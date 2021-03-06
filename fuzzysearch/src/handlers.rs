use lazy_static::lazy_static;
use prometheus::{register_histogram, register_int_counter, Histogram, IntCounter};
use std::convert::TryInto;
use tracing::{span, warn};
use tracing_futures::Instrument;
use warp::{Rejection, Reply};

use crate::models::image_query;
use crate::types::*;
use crate::{early_return, rate_limit, Pool, Tree};
use fuzzysearch_common::types::{SearchResult, SiteInfo};

lazy_static! {
    static ref IMAGE_HASH_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_hash_seconds",
        "Duration to perform an image hash operation"
    )
    .unwrap();
    static ref VIDEO_HASH_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_video_hash_seconds",
        "Duration to perform a video hash operation"
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

async fn get_field_bytes(form: warp::multipart::FormData, field: &str) -> bytes::BytesMut {
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
    let data = parts.remove(field).unwrap();

    data.stream()
        .fold(bytes::BytesMut::new(), |mut b, data| {
            b.put(data.unwrap());
            async move { b }
        })
        .await
}

#[tracing::instrument(skip(form))]
async fn hash_input(form: warp::multipart::FormData) -> i64 {
    let bytes = get_field_bytes(form, "image").await;

    let len = bytes.len();

    let _timer = IMAGE_HASH_DURATION.start_timer();
    let hash = tokio::task::spawn_blocking(move || {
        let hasher = fuzzysearch_common::get_hasher();
        let image = image::load_from_memory(&bytes).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image", len))
    .await
    .unwrap();
    drop(_timer);

    let mut buf: [u8; 8] = [0; 8];
    buf.copy_from_slice(&hash.as_bytes());

    i64::from_be_bytes(buf)
}

#[tracing::instrument(skip(form))]
async fn hash_video(form: warp::multipart::FormData) -> Option<Vec<[u8; 8]>> {
    use bytes::Buf;

    let bytes = get_field_bytes(form, "video").await;

    let _timer = VIDEO_HASH_DURATION.start_timer();
    let hashes = tokio::task::spawn_blocking(move || {
        if infer::is_video(&bytes) {
            fuzzysearch_common::video::extract_video_hashes(bytes.reader()).ok()
        } else if infer::image::is_gif(&bytes) {
            fuzzysearch_common::video::extract_gif_hashes(bytes.reader()).ok()
        } else {
            None
        }
    })
    .instrument(span!(tracing::Level::TRACE, "hashing video"))
    .await
    .unwrap();
    drop(_timer);

    hashes
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

    let num = hash_input(form).await;

    let mut items = {
        if opts.search_type == Some(ImageSearchType::Force) {
            image_query(db.clone(), tree.clone(), vec![num], 10)
                .await
                .unwrap()
        } else {
            let results = image_query(db.clone(), tree.clone(), vec![num], 0)
                .await
                .unwrap();
            if results.is_empty() && opts.search_type != Some(ImageSearchType::Exact) {
                image_query(db.clone(), tree.clone(), vec![num], 10)
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

    let results =
        early_return!(image_query(pool, tree, hashes.clone(), opts.distance.unwrap_or(10),).await);

    let resp = warp::http::Response::builder()
        .header("x-rate-limit-total-image", image_remaining.1.to_string())
        .header(
            "x-rate-limit-remaining-image",
            image_remaining.0.to_string(),
        )
        .header("content-type", "application/json")
        .body(serde_json::to_string(&results).unwrap())
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
                    submission.rating,
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
                    submission.rating,
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
                    submission.rating,
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
    } else if let Some(ref site_id) = opts.site_id {
        sqlx::query(
            "SELECT
                    submission.id,
                    submission.url,
                    submission.filename,
                    submission.file_id,
                    submission.rating,
                    artist.name,
                    hashes.id hash_id
                FROM
                    submission
                JOIN artist
                    ON artist.id = submission.artist_id
                JOIN hashes
                    ON hashes.furaffinity_id = submission.id
                WHERE
                    submission.id = $1
                LIMIT 10",
        )
        .bind(site_id)
    } else {
        return Ok(Box::new(Error::InvalidData));
    };

    let matches: Result<Vec<SearchResult>, _> = query
        .map(|row| SearchResult {
            site_id: row.get::<i32, _>("id") as i64,
            site_id_str: row.get::<i32, _>("id").to_string(),
            url: row.get("url"),
            filename: row.get("filename"),
            artists: row
                .get::<Option<String>, _>("name")
                .map(|artist| vec![artist]),
            distance: None,
            hash: None,
            searched_hash: None,
            site_info: Some(SiteInfo::FurAffinity {
                file_id: row.get("file_id"),
            }),
            rating: row
                .get::<Option<String>, _>("rating")
                .and_then(|rating| rating.parse().ok()),
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

pub async fn search_video(
    form: warp::multipart::FormData,
    _db: Pool,
    _api_key: String,
) -> Result<impl Reply, Rejection> {
    let hashes = hash_video(form).await;

    Ok(warp::reply::json(&hashes))
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
        let hasher = fuzzysearch_common::get_hasher();
        let image = image::load_from_memory(&buf).unwrap();
        hasher.hash_image(&image)
    })
    .instrument(span!(tracing::Level::TRACE, "hashing image"))
    .await
    .unwrap();
    drop(_timer);

    let hash: [u8; 8] = hash.as_bytes().try_into().unwrap();
    let num = i64::from_be_bytes(hash);

    let results = image_query(db.clone(), tree.clone(), vec![num], 3)
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
