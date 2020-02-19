use crate::models::{image_query, image_query_sync};
use crate::types::*;
use crate::{rate_limit, Pool, Tree};
use tracing::{span, warn};
use tracing_futures::Instrument;
use warp::{reject, Rejection, Reply};

fn map_bb8_err(err: bb8::RunError<tokio_postgres::Error>) -> Rejection {
    reject::custom(Error::from(err))
}

fn map_postgres_err(err: tokio_postgres::Error) -> Rejection {
    reject::custom(Error::from(err))
}

#[derive(Debug)]
enum Error {
    BB8(bb8::RunError<tokio_postgres::Error>),
    Postgres(tokio_postgres::Error),
    InvalidData,
    ApiKey,
    RateLimit,
}

impl From<bb8::RunError<tokio_postgres::Error>> for Error {
    fn from(err: bb8::RunError<tokio_postgres::Error>) -> Self {
        Error::BB8(err)
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(err: tokio_postgres::Error) -> Self {
        Error::Postgres(err)
    }
}

impl warp::reject::Reject for Error {}

#[tracing::instrument(skip(form))]
async fn hash_input(form: warp::multipart::FormData) -> (i64, img_hash::ImageHash<[u8; 8]>) {
    use bytes::BufMut;
    use futures_util::StreamExt;

    let parts: Vec<_> = form.collect().await;
    let mut parts = parts
        .into_iter()
        .map(|part| {
            let part = part.unwrap();
            (part.name().to_string(), part)
        })
        .collect::<std::collections::HashMap<_, _>>();
    let image = parts.remove("image").unwrap();

    let bytes = image
        .stream()
        .fold(bytes::BytesMut::new(), |mut b, data| {
            b.put(data.unwrap());
            async move { b }
        })
        .await;

    let len = bytes.len();

    let hash = tokio::task::spawn_blocking(move || {
        let hasher = crate::get_hasher();
        let image = image::load_from_memory(&bytes).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image", len))
    .await
    .unwrap();

    let mut buf: [u8; 8] = [0; 8];
    buf.copy_from_slice(&hash.as_bytes());

    (i64::from_be_bytes(buf), hash)
}

#[tracing::instrument(skip(_telem, form, pool, tree, api_key))]
pub async fn search_image(
    _telem: crate::Span,
    form: warp::multipart::FormData,
    opts: ImageSearchOpts,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<impl Reply, Rejection> {
    let db = pool.get().await.map_err(map_bb8_err)?;

    rate_limit!(&api_key, &db, image_limit, "image");

    let (num, hash) = hash_input(form).await;

    let mut items = {
        if opts.search_type == Some(ImageSearchType::Force) {
            image_query(
                pool.clone(),
                tree.clone(),
                vec![num],
                10,
                Some(hash.as_bytes().to_vec()),
            )
            .await
            .unwrap()
        } else {
            let results = image_query(
                pool.clone(),
                tree.clone(),
                vec![num],
                0,
                Some(hash.as_bytes().to_vec()),
            )
            .await
            .unwrap();
            if results.is_empty() && opts.search_type != Some(ImageSearchType::Exact) {
                image_query(
                    pool.clone(),
                    tree.clone(),
                    vec![num],
                    10,
                    Some(hash.as_bytes().to_vec()),
                )
                .await
                .unwrap()
            } else {
                results
            }
        }
    };

    items.sort_by(|a, b| {
        a.distance
            .unwrap_or(u64::max_value())
            .partial_cmp(&b.distance.unwrap_or(u64::max_value()))
            .unwrap()
    });

    let similarity = ImageSimilarity {
        hash: num,
        matches: items,
    };

    Ok(warp::reply::json(&similarity))
}

#[tracing::instrument(skip(_telem, form, pool, tree, api_key))]
pub async fn stream_image(
    _telem: crate::Span,
    form: warp::multipart::FormData,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<impl Reply, Rejection> {
    use futures_util::StreamExt;

    let db = pool.get().await.map_err(map_bb8_err)?;

    rate_limit!(&api_key, &db, image_limit, "image", 2);

    let (num, hash) = hash_input(form).await;

    let event_stream = image_query_sync(
        pool.clone(),
        tree,
        vec![num],
        10,
        Some(hash.as_bytes().to_vec()),
    )
    .map(sse_matches);

    Ok(warp::sse::reply(event_stream))
}

fn sse_matches(
    matches: Result<Vec<File>, tokio_postgres::Error>,
) -> Result<impl warp::sse::ServerSentEvent, core::convert::Infallible> {
    let items = matches.unwrap();

    Ok(warp::sse::json(items))
}

#[tracing::instrument(skip(_telem, form, db, tree, api_key))]
pub async fn search_hashes(
    _telem: crate::Span,
    opts: HashSearchOpts,
    db: Pool,
    tree: Tree,
    api_key: String,
) -> Result<impl Reply, Rejection> {
    let pool = db.clone();
    let db = db.get().await.map_err(map_bb8_err)?;

    let hashes: Vec<i64> = opts
        .hashes
        .split(',')
        .take(10)
        .filter_map(|hash| hash.parse::<i64>().ok())
        .collect();

    if hashes.is_empty() {
        return Err(warp::reject::custom(Error::InvalidData));
    }

    rate_limit!(&api_key, &db, image_limit, "image", hashes.len() as i16);

    let mut results = image_query_sync(
        pool,
        tree,
        hashes.clone(),
        opts.distance.unwrap_or(10),
        None,
    );
    let mut matches = Vec::new();

    while let Some(r) = results.recv().await {
        matches.extend(r.map_err(|e| warp::reject::custom(Error::Postgres(e)))?);
    }

    Ok(warp::reply::json(&matches))
}

#[tracing::instrument(skip(_telem, db, api_key))]
pub async fn search_file(
    _telem: crate::Span,
    opts: FileSearchOpts,
    db: Pool,
    api_key: String,
) -> Result<impl Reply, Rejection> {
    let db = db.get().await.map_err(map_bb8_err)?;

    rate_limit!(&api_key, &db, name_limit, "file");

    let (filter, val): (&'static str, &(dyn tokio_postgres::types::ToSql + Sync)) =
        if let Some(ref id) = opts.id {
            ("file_id = $1", id)
        } else if let Some(ref name) = opts.name {
            ("lower(filename) = lower($1)", name)
        } else if let Some(ref url) = opts.url {
            ("lower(url) = lower($1)", url)
        } else {
            return Err(warp::reject::custom(Error::InvalidData));
        };

    let query = format!(
        "SELECT
            submission.id,
            submission.url,
            submission.filename,
            submission.file_id,
            artist.name,
            hashes.id hash_id
        FROM
            submission
        JOIN artist
            ON artist.id = submission.artist_id
        JOIN hashes
            ON hashes.furaffinity_id = submission.id
        WHERE
            {}
        LIMIT 10",
        filter
    );

    let matches: Vec<_> = db
        .query::<str>(&*query, &[val])
        .instrument(span!(tracing::Level::TRACE, "waiting for db"))
        .await
        .map_err(map_postgres_err)?
        .into_iter()
        .map(|row| File {
            id: row.get("hash_id"),
            site_id: row.get::<&str, i32>("id") as i64,
            site_id_str: row.get::<&str, i32>("id").to_string(),
            url: row.get("url"),
            filename: row.get("filename"),
            artists: row
                .get::<&str, Option<String>>("name")
                .map(|artist| vec![artist]),
            distance: None,
            hash: None,
            site_info: Some(SiteInfo::FurAffinity(FurAffinityFile {
                file_id: row.get("file_id"),
            })),
            searched_hash: None,
        })
        .collect();

    Ok(warp::reply::json(&matches))
}

#[tracing::instrument]
pub async fn handle_rejection(err: Rejection) -> Result<impl Reply, std::convert::Infallible> {
    warn!("had rejection");

    let (code, message) = if err.is_not_found() {
        (
            warp::http::StatusCode::NOT_FOUND,
            "This page does not exist",
        )
    } else if let Some(err) = err.find::<Error>() {
        match err {
            Error::BB8(_inner) => (
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                "A database error occured",
            ),
            Error::Postgres(_inner) => (
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                "A database error occured",
            ),
            Error::InvalidData => (
                warp::http::StatusCode::BAD_REQUEST,
                "Unable to operate on provided data",
            ),
            Error::ApiKey => (
                warp::http::StatusCode::UNAUTHORIZED,
                "Invalid API key provided",
            ),
            Error::RateLimit => (
                warp::http::StatusCode::TOO_MANY_REQUESTS,
                "Your API token is rate limited",
            ),
        }
    } else {
        (
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An unknown error occured",
        )
    };

    let json = warp::reply::json(&ErrorMessage {
        code: code.as_u16(),
        message: message.into(),
    });

    Ok(warp::reply::with_status(json, code))
}
