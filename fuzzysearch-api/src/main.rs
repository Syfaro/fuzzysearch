use std::{borrow::Cow, str::FromStr};

use bkapi_client::BKApiClient;
use bytes::BufMut;
use hyper::StatusCode;
use poem::{error::ResponseError, listener::TcpListener, web::Data, EndpointExt, Request, Route};
use poem_openapi::{
    auth::ApiKey,
    param::{Path, Query},
    payload::{Json, Response},
    types::multipart::Upload,
    Multipart, Object, OneOf, OpenApi, OpenApiService, SecurityScheme,
};

type Pool = sqlx::PgPool;

#[derive(Clone)]
pub struct Endpoints {
    pub hash_input: String,
    pub bkapi: String,
}

struct Api;

/// Simple authentication using a static API key. Must be manually requested.
#[derive(SecurityScheme)]
#[oai(
    type = "api_key",
    key_name = "X-Api-Key",
    in = "header",
    checker = "api_checker"
)]
struct ApiKeyAuthorization(UserApiKey);

struct UserApiKey {
    id: i32,
    name: Option<String>,
    owner_email: String,
    name_limit: i16,
    image_limit: i16,
    hash_limit: i16,
}

async fn api_checker(req: &Request, api_key: ApiKey) -> Option<UserApiKey> {
    let pool: &Pool = req.data().unwrap();

    sqlx::query_file_as!(UserApiKey, "queries/lookup_api_key.sql", api_key.key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

#[derive(poem_openapi::Enum, Debug, PartialEq)]
#[oai(rename_all = "snake_case")]
enum KnownServiceName {
    Twitter,
}

#[derive(poem_openapi::Enum, Debug)]
#[oai(rename_all = "lowercase")]
enum Rating {
    General,
    Mature,
    Adult,
}

impl FromStr for Rating {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rating = match s {
            "g" | "s" | "general" => Self::General,
            "m" | "q" | "mature" => Self::Mature,
            "a" | "e" | "adult" | "explicit" => Self::Adult,
            _ => return Err(format!("unknown rating: {}", s)),
        };

        Ok(rating)
    }
}

#[derive(Object, Debug)]
#[oai(rename = "FurAffinity")]
struct FurAffinityExtra {
    file_id: i32,
}

#[derive(Object, Debug)]
#[oai(rename = "e621")]
struct E621Extra {
    sources: Vec<String>,
}

#[derive(OneOf, Debug)]
#[oai(property_name = "site")]
enum SiteExtraData {
    FurAffinity(FurAffinityExtra),
    E621(E621Extra),
}

#[derive(Object, Debug)]
struct HashLookupResult {
    site_name: String,
    site_id: i64,
    site_id_str: String,
    site_extra_data: Option<SiteExtraData>,

    url: String,
    filename: String,
    artists: Option<Vec<String>>,
    rating: Option<Rating>,
    posted_at: Option<chrono::DateTime<chrono::Utc>>,

    hash: i64,
    searched_hash: i64,
    distance: u64,
}

#[derive(serde::Serialize)]
struct HashSearch {
    searched_hash: i64,
    found_hash: i64,
    distance: u64,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("bad request: {0}")]
    BadRequest(#[from] BadRequest),
}

impl ResponseError for Error {
    fn status(&self) -> hyper::StatusCode {
        hyper::StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(Debug, thiserror::Error)]
#[error("bad request: {message}")]
struct BadRequest {
    message: Cow<'static, str>,
}

impl BadRequest {
    fn with_message<M: Into<Cow<'static, str>>>(message: M) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl ResponseError for BadRequest {
    fn status(&self) -> hyper::StatusCode {
        hyper::StatusCode::BAD_REQUEST
    }
}

#[derive(Debug, thiserror::Error)]
#[error("rate limited")]
struct RateLimited {
    bucket: String,
}

impl ResponseError for RateLimited {
    fn status(&self) -> hyper::StatusCode {
        hyper::StatusCode::TOO_MANY_REQUESTS
    }
}

/// The status of an API key's rate limit.
#[derive(Debug, PartialEq)]
pub enum RateLimit {
    /// This key is limited, we should deny the request.
    Limited,
    /// This key is available, contains the number of requests made.
    Available((i16, i16)),
}

async fn update_rate_limit(
    pool: &Pool,
    key_id: i32,
    key_group_limit: i16,
    group_name: &'static str,
    incr_by: i16,
) -> Result<RateLimit, sqlx::Error> {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp();
    let time_window = timestamp - (timestamp % 60);

    let count: i16 = sqlx::query_file_scalar!(
        "queries/update_rate_limit.sql",
        key_id,
        time_window,
        group_name,
        incr_by
    )
    .fetch_one(pool)
    .await?;

    if count > key_group_limit {
        Ok(RateLimit::Limited)
    } else {
        Ok(RateLimit::Available((
            key_group_limit - count,
            key_group_limit,
        )))
    }
}

macro_rules! rate_limit {
    ($api_key:expr, $db:expr, $limit:tt, $group:expr) => {
        rate_limit!($api_key, $db, $limit, $group, 1)
    };

    ($api_key:expr, $db:expr, $limit:tt, $group:expr, $incr_by:expr) => {{
        let rate_limit = update_rate_limit($db, $api_key.0.id, $api_key.0.$limit, $group, $incr_by)
            .await
            .map_err(Error::from)?;

        match rate_limit {
            RateLimit::Limited => {
                return Err(RateLimited {
                    bucket: $group.to_string(),
                }
                .into())
            }
            RateLimit::Available(count) => count,
        }
    }};
}

async fn lookup_hashes(
    pool: &Pool,
    bkapi: &BKApiClient,
    hashes: &[i64],
    distance: u64,
) -> Result<Vec<HashLookupResult>, Error> {
    if distance > 10 {
        return Err(BadRequest::with_message(format!("distance too large: {}", distance)).into());
    }

    let index_hashes: Vec<_> = bkapi
        .search_many(hashes, distance)
        .await?
        .into_iter()
        .flat_map(|results| {
            let hash = results.hash;

            results.hashes.into_iter().map(move |result| HashSearch {
                searched_hash: hash,
                found_hash: result.hash,
                distance: result.distance,
            })
        })
        .collect();

    let data = serde_json::to_value(index_hashes)?;

    let results = sqlx::query_file!("queries/lookup_hashes.sql", data)
        .map(|row| {
            let site_extra_data = match row.site.as_deref() {
                Some("FurAffinity") => Some(SiteExtraData::FurAffinity(FurAffinityExtra {
                    file_id: row.file_id.unwrap_or(-1),
                })),
                Some("e621") => Some(SiteExtraData::E621(E621Extra {
                    sources: row.sources.unwrap_or_default(),
                })),
                _ => None,
            };

            HashLookupResult {
                site_name: row.site.unwrap_or_default(),
                site_id: row.id.unwrap_or_default(),
                site_id_str: row.id.unwrap_or_default().to_string(),
                site_extra_data,
                url: row.url.unwrap_or_default(),
                filename: row.filename.unwrap_or_default(),
                artists: row.artists,
                posted_at: row.posted_at,
                rating: row.rating.and_then(|rating| rating.parse().ok()),
                hash: row.hash.unwrap_or_default(),
                searched_hash: row.searched_hash.unwrap_or_default(),
                distance: row.distance.unwrap_or_default() as u64,
            }
        })
        .fetch_all(pool)
        .await?;

    Ok(results)
}

#[derive(Debug, Multipart)]
struct ImageSearchPayload {
    image: Upload,
}

async fn hash_input(
    client: &reqwest::Client,
    hash_input_endpoint: &str,
    image: reqwest::Body,
) -> Result<i64, Error> {
    let part = reqwest::multipart::Part::stream(image);
    let form = reqwest::multipart::Form::new().part("image", part);

    let resp = client
        .post(hash_input_endpoint)
        .multipart(form)
        .send()
        .await?;

    if resp.status() != StatusCode::OK {
        return Err(BadRequest::with_message("invalid image").into());
    }

    match resp.text().await?.parse() {
        Ok(hash) => Ok(hash),
        Err(_err) => Err(BadRequest::with_message("invalid image").into()),
    }
}

#[derive(poem_openapi::Enum, Debug, PartialEq)]
#[oai(rename_all = "lowercase")]
enum ImageSearchType {
    Force,
    Close,
    Exact,
}

#[derive(Object, Debug)]
struct ImageSearchResult {
    hash: i64,
    matches: Vec<HashLookupResult>,
}

#[derive(Object, Debug)]
struct FurAffinityFile {
    id: i32,
    url: Option<String>,
    filename: Option<String>,
    file_id: Option<i32>,
    rating: Option<Rating>,
    posted_at: Option<chrono::DateTime<chrono::Utc>>,
    artist: Option<String>,
    hash: Option<i64>,
}

#[OpenApi]
impl Api {
    /// Lookup images by hash
    ///
    /// Perform a lookup for up to 10 given hashes.
    #[oai(path = "/hashes", method = "get")]
    async fn hashes(
        &self,
        pool: Data<&Pool>,
        bkapi: Data<&BKApiClient>,
        auth: ApiKeyAuthorization,
        hashes: Query<String>,
        distance: Query<Option<u64>>,
    ) -> poem::Result<Response<Json<Vec<HashLookupResult>>>> {
        let hashes: Vec<i64> = hashes
            .0
            .split(',')
            .take(10)
            .filter_map(|hash| hash.parse().ok())
            .collect();

        let image_remaining = rate_limit!(auth, pool.0, image_limit, "image", hashes.len() as i16);

        if hashes.is_empty() {
            return Err(BadRequest::with_message("hashes must be provided").into());
        }

        let results = lookup_hashes(&pool, &bkapi, &hashes, distance.unwrap_or(3)).await?;

        let resp = Response::new(Json(results))
            .header("x-rate-limit-total-image", image_remaining.1)
            .header("x-rate-limit-remaining-image", image_remaining.0);

        Ok(resp)
    }

    /// Lookup images by image
    ///
    /// Perform a lookup with a given image.
    #[oai(path = "/image", method = "post")]
    async fn image(
        &self,
        pool: Data<&Pool>,
        bkapi: Data<&BKApiClient>,
        client: Data<&reqwest::Client>,
        endpoints: Data<&Endpoints>,
        auth: ApiKeyAuthorization,
        search_type: Query<Option<ImageSearchType>>,
        payload: ImageSearchPayload,
    ) -> poem::Result<Response<Json<ImageSearchResult>>> {
        let image_remaining = rate_limit!(auth, pool.0, image_limit, "image");
        let hash_remaining = rate_limit!(auth, pool.0, hash_limit, "hash");

        let stream = tokio_util::io::ReaderStream::new(payload.image.into_async_read());
        let body = reqwest::Body::wrap_stream(stream);

        let hash = hash_input(&client, &endpoints.hash_input, body).await?;

        let search_type = search_type.0.unwrap_or(ImageSearchType::Close);
        let hashes = vec![hash];

        let mut results = {
            if search_type == ImageSearchType::Force {
                lookup_hashes(pool.0, bkapi.0, &hashes, 10).await?
            } else {
                let results = lookup_hashes(pool.0, bkapi.0, &hashes, 0).await?;

                if results.is_empty() && search_type != ImageSearchType::Exact {
                    lookup_hashes(pool.0, bkapi.0, &hashes, 10).await?
                } else {
                    results
                }
            }
        };

        results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

        let resp = Response::new(Json(ImageSearchResult {
            hash,
            matches: results,
        }))
        .header("x-image-hash", hash)
        .header("x-rate-limit-total-image", image_remaining.1)
        .header("x-rate-limit-remaining-image", image_remaining.0)
        .header("x-rate-limit-total-hash", hash_remaining.1)
        .header("x-rate-limit-remaining-hash", hash_remaining.0);

        Ok(resp)
    }

    /// Lookup images by image URL
    ///
    /// Perform a lookup for an image at the given URL. Image may not exceed 10MB.
    #[oai(path = "/url", method = "get")]
    async fn url(
        &self,
        pool: Data<&Pool>,
        bkapi: Data<&BKApiClient>,
        client: Data<&reqwest::Client>,
        endpoints: Data<&Endpoints>,
        auth: ApiKeyAuthorization,
        url: Query<String>,
        distance: Query<Option<u64>>,
    ) -> poem::Result<Response<Json<ImageSearchResult>>> {
        let image_remaining = rate_limit!(auth, pool.0, image_limit, "image");
        let hash_remaining = rate_limit!(auth, pool.0, hash_limit, "hash");

        let mut resp = client.get(&url.0).send().await.map_err(Error::from)?;

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
            return Err(BadRequest::with_message(format!(
                "image too large: {} bytes",
                content_length
            ))
            .into());
        }

        let mut buf = bytes::BytesMut::with_capacity(content_length);

        while let Some(chunk) = resp.chunk().await.map_err(Error::from)? {
            if buf.len() + chunk.len() > 10_000_000 {
                return Err(BadRequest::with_message(format!(
                    "image too large: {} bytes",
                    content_length
                ))
                .into());
            }

            buf.put(chunk);
        }

        let body = reqwest::Body::from(buf.to_vec());
        let hash = hash_input(&client, &endpoints.hash_input, body).await?;

        let results = lookup_hashes(pool.0, bkapi.0, &[hash], distance).await?;

        let resp = Response::new(Json(ImageSearchResult {
            hash,
            matches: results,
        }))
        .header("x-image-hash", hash)
        .header("x-rate-limit-total-image", image_remaining.1)
        .header("x-rate-limit-remaining-image", image_remaining.0)
        .header("x-rate-limit-total-hash", hash_remaining.1)
        .header("x-rate-limit-remaining-hash", hash_remaining.0);

        Ok(resp)
    }

    /// Lookup FurAffinity submission by File ID
    #[oai(path = "/furaffinity/file_id", method = "get")]
    async fn furaffinity_data(
        &self,
        pool: Data<&Pool>,
        auth: ApiKeyAuthorization,
        file_id: Query<i32>,
    ) -> poem::Result<Response<Json<Vec<FurAffinityFile>>>> {
        let file_remaining = rate_limit!(auth, pool.0, image_limit, "file");

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
            .fetch_all(pool.0)
            .await
            .map_err(Error::from)?;

        let resp = Response::new(Json(matches))
            .header("x-rate-limit-total-file", file_remaining.1)
            .header("x-rate-limit-remaining-file", file_remaining.0);

        Ok(resp)
    }

    /// Check if a handle is known for a given service
    ///
    /// If the handle is known, the associated media items should be available
    /// in the search index.
    #[oai(path = "/known/:service", method = "get")]
    async fn known_service(
        &self,
        pool: Data<&Pool>,
        service: Path<KnownServiceName>,
        handle: Query<String>,
    ) -> poem::Result<Json<bool>> {
        let handle_exists = match service.0 {
            KnownServiceName::Twitter => {
                sqlx::query_file_scalar!("queries/handle_twitter.sql", handle.0)
                    .fetch_one(pool.0)
                    .await
                    .map_err(poem::error::InternalServerError)?
            }
        };

        Ok(Json(handle_exists))
    }
}

#[tokio::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch-api");
    fuzzysearch_common::trace::serve_metrics().await;

    let server_endpoint =
        std::env::var("SERVER_ENDPOINT").unwrap_or_else(|_err| "http://localhost:8080".to_string());

    let database_url = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Unable to create Postgres pool");

    let endpoints = Endpoints {
        hash_input: std::env::var("ENDPOINT_HASH_INPUT").expect("Missing ENDPOINT_HASH_INPUT"),
        bkapi: std::env::var("ENDPOINT_BKAPI").expect("Missing ENDPOINT_BKAPI"),
    };

    let bkapi = BKApiClient::new(&endpoints.bkapi);

    let cors = poem::middleware::Cors::new()
        .allow_methods([poem::http::Method::GET, poem::http::Method::POST]);

    let api_service = OpenApiService::new(Api, "FuzzySearch", "1.0").server(server_endpoint);
    let api_spec_endpoint = api_service.spec_endpoint();

    let docs = api_service.swagger_ui();
    let app = Route::new()
        .nest("/", api_service)
        .nest("/docs", docs)
        .at("/openapi.json", api_spec_endpoint)
        .data(pool)
        .data(bkapi)
        .data(endpoints)
        .data(reqwest::Client::new())
        .with(poem::middleware::Tracing)
        .with(poem::middleware::OpenTelemetryMetrics::new())
        .with(cors);

    poem::Server::new(TcpListener::bind("0.0.0.0:8080"))
        .run(app)
        .await
        .unwrap();
}
