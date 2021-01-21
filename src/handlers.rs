use crate::models::{image_query, image_query_sync};
use crate::types::*;
use crate::{early_return, rate_limit, Pool, Tree};
use std::convert::TryInto;
use tracing::{span, warn};
use tracing_futures::Instrument;
use warp::{Rejection, Reply};

#[derive(Debug)]
enum Error {
    BB8(bb8::RunError<tokio_postgres::Error>),
    Postgres(tokio_postgres::Error),
    InvalidData,
    ApiKey,
    RateLimit,
}

impl warp::Reply for Error {
    fn into_response(self) -> warp::reply::Response {
        let msg = match self {
            Error::BB8(_) | Error::Postgres(_) => ErrorMessage {
                code: 500,
                message: "Internal server error".to_string(),
            },
            Error::InvalidData => ErrorMessage {
                code: 400,
                message: "Invalid data provided".to_string(),
            },
            Error::ApiKey => ErrorMessage {
                code: 401,
                message: "Invalid API key".to_string(),
            },
            Error::RateLimit => ErrorMessage {
                code: 429,
                message: "Too many requests".to_string(),
            },
        };

        let body = hyper::body::Body::from(serde_json::to_string(&msg).unwrap());

        warp::http::Response::builder()
            .status(msg.code)
            .body(body)
            .unwrap()
    }
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

pub async fn search_image(
    form: warp::multipart::FormData,
    opts: ImageSearchOpts,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let db = early_return!(pool.get().await);

    rate_limit!(&api_key, &db, image_limit, "image");
    rate_limit!(&api_key, &db, hash_limit, "hash");

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

    Ok(Box::new(warp::reply::json(&similarity)))
}

pub async fn stream_image(
    form: warp::multipart::FormData,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let db = early_return!(pool.get().await);

    rate_limit!(&api_key, &db, image_limit, "image", 2);
    rate_limit!(&api_key, &db, hash_limit, "hash");

    let (num, hash) = hash_input(form).await;

    let mut query = image_query_sync(
        pool.clone(),
        tree,
        vec![num],
        10,
        Some(hash.as_bytes().to_vec()),
    );

    let event_stream = async_stream::stream! {
        while let Some(result) = query.recv().await {
            yield sse_matches(result);
        }
    };

    Ok(Box::new(warp::sse::reply(event_stream)))
}

fn sse_matches(
    matches: Result<Vec<File>, tokio_postgres::Error>,
) -> Result<warp::sse::Event, core::convert::Infallible> {
    let items = matches.unwrap();

    Ok(warp::sse::Event::default().json_data(items).unwrap())
}

pub async fn search_hashes(
    opts: HashSearchOpts,
    db: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let pool = db.clone();
    let db = early_return!(db.get().await);

    let hashes: Vec<i64> = opts
        .hashes
        .split(',')
        .take(10)
        .filter_map(|hash| hash.parse::<i64>().ok())
        .collect();

    if hashes.is_empty() {
        return Ok(Box::new(Error::InvalidData));
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
        matches.extend(early_return!(r));
    }

    Ok(Box::new(warp::reply::json(&matches)))
}

pub async fn search_file(
    opts: FileSearchOpts,
    db: Pool,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let db = early_return!(db.get().await);

    rate_limit!(&api_key, &db, name_limit, "file");

    let (filter, val): (&'static str, &(dyn tokio_postgres::types::ToSql + Sync)) =
        if let Some(ref id) = opts.id {
            ("file_id = $1", id)
        } else if let Some(ref name) = opts.name {
            ("lower(filename) = lower($1)", name)
        } else if let Some(ref url) = opts.url {
            ("lower(url) = lower($1)", url)
        } else {
            return Ok(Box::new(Error::InvalidData));
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

    let matches: Vec<_> = early_return!(
        db.query::<str>(&*query, &[val])
            .instrument(span!(tracing::Level::TRACE, "waiting for db"))
            .await
    )
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

    Ok(Box::new(warp::reply::json(&matches)))
}

pub async fn check_handle(opts: HandleOpts, db: Pool) -> Result<Box<dyn Reply>, Rejection> {
    let db = early_return!(db.get().await);

    let exists = if let Some(handle) = opts.twitter {
        !early_return!(
            db.query(
                "SELECT 1 FROM twitter_user WHERE lower(data->>'screen_name') = lower($1)",
                &[&handle],
            )
            .await
        )
        .is_empty()
    } else {
        false
    };

    Ok(Box::new(warp::reply::json(&exists)))
}

pub async fn search_image_by_url(
    opts: URLSearchOpts,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let url = opts.url;

    let db = early_return!(pool.get().await);

    let image_remaining = rate_limit!(&api_key, &db, image_limit, "image");
    let hash_remaining = rate_limit!(&api_key, &db, hash_limit, "hash");

    let resp = match reqwest::get(&url).await {
        Ok(resp) => resp,
        Err(err) => return Ok(Box::new(warp::reply::json(&format!("Error: {}", err)))),
    };

    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => return Ok(Box::new(warp::reply::json(&format!("Error: {}", err)))),
    };

    let hash = tokio::task::spawn_blocking(move || {
        let hasher = crate::get_hasher();
        let image = image::load_from_memory(&bytes).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image"))
    .await
    .unwrap();

    let hash: [u8; 8] = hash.as_bytes().try_into().unwrap();
    let num = i64::from_be_bytes(hash);

    let results = image_query(
        pool.clone(),
        tree.clone(),
        vec![num],
        3,
        Some(hash.to_vec()),
    )
    .await
    .unwrap();

    let resp = warp::http::Response::builder()
        .header("x-image-hash", num.to_string())
        .header("x-rate-limit-total-image", image_remaining.1.to_string())
        .header(
            "x-rate-limit-remaining-image",
            image_remaining.0.to_string(),
        )
        .header("x-rate-limit-total-hash", hash_remaining.1.to_string())
        .header("x-rate-limit-remaining-hash", hash_remaining.0.to_string())
        .header("content-type", "application/json")
        .body(serde_json::to_string(&results).unwrap())
        .unwrap();

    Ok(Box::new(resp))
}

#[tracing::instrument]
pub async fn handle_rejection(err: Rejection) -> Result<Box<dyn Reply>, std::convert::Infallible> {
    warn!("had rejection");

    let (code, message) = if err.is_not_found() {
        (
            warp::http::StatusCode::NOT_FOUND,
            "This page does not exist",
        )
    } else if err.find::<warp::reject::InvalidQuery>().is_some() {
        return Ok(Box::new(Error::InvalidData) as Box<dyn Reply>)
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        return Ok(Box::new(Error::InvalidData) as Box<dyn Reply>)
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

    Ok(Box::new(warp::reply::with_status(json, code)))
}
