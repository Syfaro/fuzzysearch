use bkapi_client::BKApiClient;
use bytes::BufMut;
use poem_openapi::{
    param::{Path, Query},
    payload::{Json, Response},
    types::ToJSON,
    ApiResponse, Object,
};

use crate::{
    hash_input, lookup_hashes, rate_limit, Endpoints, FurAffinityFile, HashLookupResult,
    ImageSearchPayload, ImageSearchResult, ImageSearchType, KnownServiceName, Pool,
    ResponseRateLimitHeaders,
};

mod auth;

pub(crate) use auth::ApiKeyAuthorization;

#[derive(Object)]
pub(crate) struct RateLimitResponse {
    bucket: String,
    retry_after: i32,
}

#[derive(Object)]
pub(crate) struct BadRequestResponse {
    message: String,
}

#[derive(ApiResponse)]
pub(crate) enum RateLimitedResponse<T, E = BadRequestResponse>
where
    T: ToJSON,
    E: ToJSON,
{
    /// The request was successful.
    #[oai(status = 200)]
    Available(Json<T>),

    /// The request was not valid. A description can be found within the
    /// resulting message.
    #[oai(status = 400)]
    BadRequest(Json<E>),

    /// The API key has exhaused the maximum permissible requests for the
    /// current time window. The `retry_after` field contains the number of
    /// seconds before a request is likely to succeed.
    #[oai(status = 429)]
    Limited(Json<RateLimitResponse>),
}

impl<T, E> RateLimitedResponse<T, E>
where
    T: ToJSON,
    E: ToJSON,
{
    pub(crate) fn available(json: T) -> Response<Self> {
        Self::Available(Json(json)).response()
    }

    pub(crate) fn limited<S: ToString>(bucket: S, retry_after: i32) -> Response<Self> {
        Self::Limited(Json(RateLimitResponse {
            bucket: bucket.to_string(),
            retry_after,
        }))
        .response()
    }

    fn response(self) -> Response<Self> {
        Response::new(self)
    }
}

impl<T> RateLimitedResponse<T, BadRequestResponse>
where
    T: ToJSON,
{
    pub(crate) fn bad_request<M: ToString>(message: M) -> Response<Self> {
        Self::BadRequest(Json(BadRequestResponse {
            message: message.to_string(),
        }))
        .response()
    }
}

#[tracing::instrument(err, skip(pool, bkapi, auth, hashes, distance), fields(hashes = %hashes.0, distance = ?distance.0))]
pub(crate) async fn hashes(
    pool: &Pool,
    bkapi: &BKApiClient,
    auth: ApiKeyAuthorization,
    hashes: Query<String>,
    distance: Query<Option<u64>>,
) -> poem::Result<Response<RateLimitedResponse<Vec<HashLookupResult>>>> {
    let hashes: Vec<i64> = hashes
        .0
        .split(',')
        .take(10)
        .filter_map(|hash| hash.parse().ok())
        .collect();

    if hashes.is_empty() {
        return Ok(RateLimitedResponse::bad_request("hashes must be provided"));
    }

    let image_remaining = rate_limit!(auth, pool, image_limit, "image", hashes.len() as i16);

    let results = lookup_hashes(pool, bkapi, &hashes, distance.unwrap_or(3)).await?;

    let resp =
        RateLimitedResponse::available(results).inject_rate_limit_headers("image", image_remaining);

    Ok(resp)
}

#[tracing::instrument(err, skip(pool, bkapi, client, endpoints, auth, search_type, payload))]
pub(crate) async fn image(
    pool: &Pool,
    bkapi: &BKApiClient,
    client: &reqwest::Client,
    endpoints: &Endpoints,
    auth: ApiKeyAuthorization,
    search_type: Query<Option<ImageSearchType>>,
    payload: ImageSearchPayload,
) -> poem::Result<Response<RateLimitedResponse<ImageSearchResult>>> {
    let image_remaining = rate_limit!(auth, pool, image_limit, "image");
    let hash_remaining = rate_limit!(auth, pool, hash_limit, "hash");

    let stream = tokio_util::io::ReaderStream::new(payload.image.into_async_read());
    let body = reqwest::Body::wrap_stream(stream);

    let hash = hash_input(client, &endpoints.hash_input, body).await?;

    let search_type = search_type.0.unwrap_or(ImageSearchType::Close);
    let hashes = vec![hash];

    let mut results = {
        if search_type == ImageSearchType::Force {
            tracing::debug!("search type is force, starting with distance of 10");
            lookup_hashes(pool, bkapi, &hashes, 10).await?
        } else {
            tracing::debug!("close or exact search type, starting with distance of 0");
            let results = lookup_hashes(pool, bkapi, &hashes, 0).await?;

            if results.is_empty() && search_type != ImageSearchType::Exact {
                tracing::debug!("results were empty and type is not force, expanding search");
                lookup_hashes(pool, bkapi, &hashes, 10).await?
            } else {
                tracing::debug!("results were not empty or search type was force, ending search");
                results
            }
        }
    };

    tracing::info!("search ended with {} results", results.len());

    results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

    let resp = RateLimitedResponse::available(ImageSearchResult {
        hash,
        matches: results,
    })
    .header("x-image-hash", hash)
    .inject_rate_limit_headers("image", image_remaining)
    .inject_rate_limit_headers("hash", hash_remaining);

    Ok(resp)
}

#[tracing::instrument(err, skip(pool, bkapi, client, endpoints, auth, url, distance), fields(url = %url.0, distance = ?distance.0))]
pub(crate) async fn url(
    pool: &Pool,
    bkapi: &BKApiClient,
    client: &reqwest::Client,
    endpoints: &Endpoints,
    auth: ApiKeyAuthorization,
    url: Query<String>,
    distance: Query<Option<u64>>,
) -> poem::Result<Response<RateLimitedResponse<ImageSearchResult>>> {
    let image_remaining = rate_limit!(auth, pool, image_limit, "image");
    let hash_remaining = rate_limit!(auth, pool, hash_limit, "hash");

    let mut resp = client
        .get(&url.0)
        .send()
        .await
        .map_err(crate::Error::from)?;

    let distance = distance.unwrap_or(3);

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
        return Ok(RateLimitedResponse::bad_request(format!(
            "image too large: {} bytes",
            content_length
        )));
    }

    let mut buf = bytes::BytesMut::with_capacity(content_length);

    while let Some(chunk) = resp.chunk().await.map_err(crate::Error::from)? {
        if buf.len() + chunk.len() > 10_000_000 {
            return Ok(RateLimitedResponse::bad_request(format!(
                "image too large: {}+ bytes",
                buf.len() + chunk.len()
            )));
        }

        buf.put(chunk);
    }

    let body = reqwest::Body::from(buf.to_vec());
    let hash = hash_input(client, &endpoints.hash_input, body).await?;

    let results = lookup_hashes(pool, bkapi, &[hash], distance).await?;

    let resp = RateLimitedResponse::available(ImageSearchResult {
        hash,
        matches: results,
    })
    .header("x-image-hash", hash)
    .inject_rate_limit_headers("image", image_remaining)
    .inject_rate_limit_headers("hash", hash_remaining);

    Ok(resp)
}

#[tracing::instrument(err, skip(pool, auth, file_id), fields(file_id = %file_id.0))]
pub(crate) async fn furaffinity_data(
    pool: &Pool,
    auth: ApiKeyAuthorization,
    file_id: Query<i32>,
) -> poem::Result<Response<RateLimitedResponse<Vec<FurAffinityFile>>>> {
    let file_remaining = rate_limit!(auth, pool, image_limit, "file");

    let matches = sqlx::query_file!("queries/lookup_furaffinity_file_id.sql", file_id.0)
        .map(|row| FurAffinityFile {
            id: row.id,
            url: row.url,
            filename: row.filename,
            file_id: row.file_id,
            rating: row.rating.and_then(|rating| rating.parse().ok()),
            posted_at: row.posted_at,
            artist: Some(row.artist),
            hash: row.hash,
        })
        .fetch_all(pool)
        .await
        .map_err(crate::Error::from)?;

    let resp =
        RateLimitedResponse::available(matches).inject_rate_limit_headers("file", file_remaining);

    Ok(resp)
}

#[tracing::instrument(err, skip(pool, service, handle), fields(service = %service.0, handle = %handle.0))]
pub(crate) async fn known_service(
    pool: &Pool,
    service: Path<KnownServiceName>,
    handle: Query<String>,
) -> poem::Result<Json<bool>> {
    let handle_exists = match service.0 {
        KnownServiceName::Twitter => {
            sqlx::query_file_scalar!("queries/handle_twitter.sql", handle.0)
                .fetch_one(pool)
                .await
                .map_err(poem::error::InternalServerError)?
        }
    };

    Ok(Json(handle_exists))
}
