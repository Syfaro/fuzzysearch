use crate::models::{image_query, image_query_sync};
use crate::types::*;
use crate::{early_return, rate_limit, Pool, Tree};
use lazy_static::lazy_static;
use prometheus::{register_histogram, register_int_counter, Histogram, IntCounter};
use std::convert::TryInto;
use tracing::{span, warn};
use tracing_futures::Instrument;
use warp::{Rejection, Reply};

lazy_static! {
    static ref IMAGE_HASH_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_hash_seconds",
        "Duration to perform an image hash operation"
    )
    .unwrap();
    static ref IMAGE_URL_DOWNLOAD_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_url_download_seconds",
        "Duration to download an image from a provided URL"
    )
    .unwrap();
    static ref UNHANDLED_REJECTIONS: IntCounter = register_int_counter!(
        "fuzzysearch_api_unhandled_rejections_count",
        "Number of unhandled HTTP rejections"
    )
    .unwrap();
}

#[derive(Debug)]
enum Error {
    Postgres(sqlx::Error),
    Reqwest(reqwest::Error),
    InvalidData,
    InvalidImage,
    ApiKey,
    RateLimit,
}

impl warp::Reply for Error {
    fn into_response(self) -> warp::reply::Response {
        let msg = match self {
            Error::Postgres(_) | Error::Reqwest(_) => ErrorMessage {
                code: 500,
                message: "Internal server error".to_string(),
            },
            Error::InvalidData => ErrorMessage {
                code: 400,
                message: "Invalid data provided".to_string(),
            },
            Error::InvalidImage => ErrorMessage {
                code: 400,
                message: "Invalid image provided".to_string(),
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

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Error::Postgres(err)
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Reqwest(err)
    }
}

#[tracing::instrument(skip(form))]
async fn hash_input(form: warp::multipart::FormData) -> (i64, img_hash::ImageHash<[u8; 8]>) {
    use bytes::BufMut;
    use futures::StreamExt;

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

    let _timer = IMAGE_HASH_DURATION.start_timer();
    let hash = tokio::task::spawn_blocking(move || {
        let hasher = crate::get_hasher();
        let image = image::load_from_memory(&bytes).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image", len))
    .await
    .unwrap();
    drop(_timer);

    let mut buf: [u8; 8] = [0; 8];
    buf.copy_from_slice(&hash.as_bytes());

    (i64::from_be_bytes(buf), hash)
}

pub async fn search_image(
    form: warp::multipart::FormData,
    opts: ImageSearchOpts,
    db: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    let image_remaining = rate_limit!(&api_key, &db, image_limit, "image");
    let hash_remaining = rate_limit!(&api_key, &db, hash_limit, "hash");

    let (num, hash) = hash_input(form).await;

    let mut items = {
        if opts.search_type == Some(ImageSearchType::Force) {
            image_query(
                db.clone(),
                tree.clone(),
                vec![num],
                10,
                Some(hash.as_bytes().to_vec()),
            )
            .await
            .unwrap()
        } else {
            let results = image_query(
                db.clone(),
                tree.clone(),
                vec![num],
                0,
                Some(hash.as_bytes().to_vec()),
            )
            .await
            .unwrap();
            if results.is_empty() && opts.search_type != Some(ImageSearchType::Exact) {
                image_query(
                    db.clone(),
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
        .body(serde_json::to_string(&similarity).unwrap())
        .unwrap();

    Ok(Box::new(resp))
}

pub async fn stream_image(
    form: warp::multipart::FormData,
    pool: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    rate_limit!(&api_key, &pool, image_limit, "image", 2);
    rate_limit!(&api_key, &pool, hash_limit, "hash");

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

#[allow(clippy::unnecessary_wraps)]
fn sse_matches(
    matches: Result<Vec<File>, sqlx::Error>,
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

    let hashes: Vec<i64> = opts
        .hashes
        .split(',')
        .take(10)
        .filter_map(|hash| hash.parse::<i64>().ok())
        .collect();

    if hashes.is_empty() {
        return Ok(Box::new(Error::InvalidData));
    }

    let image_remaining = rate_limit!(&api_key, &db, image_limit, "image", hashes.len() as i16);

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

    let resp = warp::http::Response::builder()
        .header("x-rate-limit-total-image", image_remaining.1.to_string())
        .header(
            "x-rate-limit-remaining-image",
            image_remaining.0.to_string(),
        )
        .header("content-type", "application/json")
        .body(serde_json::to_string(&matches).unwrap())
        .unwrap();

    Ok(Box::new(resp))
}

pub async fn search_file(
    opts: FileSearchOpts,
    db: Pool,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    use sqlx::Row;

    let file_remaining = rate_limit!(&api_key, &db, name_limit, "file");

    let query = if let Some(ref id) = opts.id {
        sqlx::query(
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
                    file_id = $1
                LIMIT 10",
        )
        .bind(id)
    } else if let Some(ref name) = opts.name {
        sqlx::query(
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
                    lower(filename) = lower($1)
                LIMIT 10",
        )
        .bind(name)
    } else if let Some(ref url) = opts.url {
        sqlx::query(
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
                    lower(url) = lower($1)
                LIMIT 10",
        )
        .bind(url)
    } else {
        return Ok(Box::new(Error::InvalidData));
    };

    let matches: Result<Vec<File>, _> = query
        .map(|row| File {
            id: row.get("hash_id"),
            site_id: row.get::<i32, _>("id") as i64,
            site_id_str: row.get::<i32, _>("id").to_string(),
            url: row.get("url"),
            filename: row.get("filename"),
            artists: row
                .get::<Option<String>, _>("name")
                .map(|artist| vec![artist]),
            distance: None,
            hash: None,
            site_info: Some(SiteInfo::FurAffinity(FurAffinityFile {
                file_id: row.get("file_id"),
            })),
            searched_hash: None,
        })
        .fetch_all(&db)
        .await;

    let matches = early_return!(matches);

    let resp = warp::http::Response::builder()
        .header("x-rate-limit-total-file", file_remaining.1.to_string())
        .header("x-rate-limit-remaining-file", file_remaining.0.to_string())
        .header("content-type", "application/json")
        .body(serde_json::to_string(&matches).unwrap())
        .unwrap();

    Ok(Box::new(resp))
}

pub async fn check_handle(opts: HandleOpts, db: Pool) -> Result<Box<dyn Reply>, Rejection> {
    let exists = if let Some(handle) = opts.twitter {
        let result = sqlx::query_scalar!("SELECT exists(SELECT 1 FROM twitter_user WHERE lower(data->>'screen_name') = lower($1))", handle)
            .fetch_optional(&db)
            .await
            .map(|row| row.flatten().unwrap_or(false));

        early_return!(result)
    } else {
        false
    };

    Ok(Box::new(warp::reply::json(&exists)))
}

pub async fn search_image_by_url(
    opts: UrlSearchOpts,
    db: Pool,
    tree: Tree,
    api_key: String,
) -> Result<Box<dyn Reply>, Rejection> {
    use bytes::BufMut;

    let url = opts.url;

    let image_remaining = rate_limit!(&api_key, &db, image_limit, "image");
    let hash_remaining = rate_limit!(&api_key, &db, hash_limit, "hash");

    let _timer = IMAGE_URL_DOWNLOAD_DURATION.start_timer();

    let mut resp = match reqwest::get(&url).await {
        Ok(resp) => resp,
        Err(_err) => return Ok(Box::new(Error::InvalidImage)),
    };

    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|len| {
            String::from_utf8_lossy(len.as_bytes())
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0);

    if content_length > 10_000_000 {
        return Ok(Box::new(Error::InvalidImage));
    }

    let mut buf = bytes::BytesMut::with_capacity(content_length);

    while let Some(chunk) = early_return!(resp.chunk().await) {
        if buf.len() + chunk.len() > 10_000_000 {
            return Ok(Box::new(Error::InvalidImage));
        }

        buf.put(chunk);
    }

    drop(_timer);

    let _timer = IMAGE_HASH_DURATION.start_timer();
    let hash = tokio::task::spawn_blocking(move || {
        let hasher = crate::get_hasher();
        let image = image::load_from_memory(&buf).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image"))
    .await
    .unwrap();
    drop(_timer);

    let hash: [u8; 8] = hash.as_bytes().try_into().unwrap();
    let num = i64::from_be_bytes(hash);

    let results = image_query(db.clone(), tree.clone(), vec![num], 3, Some(hash.to_vec()))
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

    UNHANDLED_REJECTIONS.inc();

    let (code, message) = if err.is_not_found() {
        (
            warp::http::StatusCode::NOT_FOUND,
            "This page does not exist",
        )
    } else if err.find::<warp::reject::InvalidQuery>().is_some() {
        return Ok(Box::new(Error::InvalidData) as Box<dyn Reply>);
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        return Ok(Box::new(Error::InvalidData) as Box<dyn Reply>);
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
