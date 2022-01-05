use std::{borrow::Cow, fmt::Display, str::FromStr};

use api::ApiKeyAuthorization;
use bkapi_client::BKApiClient;
use hyper::StatusCode;
use lazy_static::lazy_static;
use poem::{error::ResponseError, listener::TcpListener, web::Data, EndpointExt, Route};
use poem_openapi::{
    param::{Path, Query},
    payload::{Json, Response},
    types::multipart::Upload,
    Multipart, Object, OneOf, OpenApi, OpenApiService,
};
use prometheus::{register_histogram, register_int_counter_vec, Histogram, IntCounterVec};

mod api;

type Pool = sqlx::PgPool;

lazy_static! {
    static ref RATE_LIMIT_ATTEMPTS: IntCounterVec = register_int_counter_vec!(
        "fuzzysearch_api_rate_limit_attempts_count",
        "Number of attempts on each rate limit bucket",
        &["bucket", "status"]
    )
    .unwrap();
    static ref IMAGE_QUERY_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_query_seconds",
        "Duration to perform an image lookup query"
    )
    .unwrap();
    static ref IMAGE_HASH_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_hash_seconds",
        "Duration to send image for hashing"
    )
    .unwrap();
    static ref IMAGE_URL_DOWNLOAD_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_url_download_seconds",
        "Duration to download an image from a provided URL"
    )
    .unwrap();
}

#[derive(Clone)]
pub struct Endpoints {
    pub hash_input: String,
    pub bkapi: String,
}

struct Api;

#[derive(poem_openapi::Enum, Debug, PartialEq)]
#[oai(rename_all = "snake_case")]
enum KnownServiceName {
    Twitter,
}

impl Display for KnownServiceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Twitter => write!(f, "Twitter"),
        }
    }
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
        match self {
            Self::BadRequest(_) => hyper::StatusCode::BAD_REQUEST,
            _ => hyper::StatusCode::INTERNAL_SERVER_ERROR,
        }
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
    bucket_name: &'static str,
    incr_by: i16,
) -> Result<RateLimit, sqlx::Error> {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp();
    let time_window = timestamp - (timestamp % 60);

    let count: i16 = sqlx::query_file_scalar!(
        "queries/update_rate_limit.sql",
        key_id,
        time_window,
        bucket_name,
        incr_by
    )
    .fetch_one(pool)
    .await?;

    if count > key_group_limit {
        RATE_LIMIT_ATTEMPTS
            .with_label_values(&[bucket_name, "limited"])
            .inc();

        Ok(RateLimit::Limited)
    } else {
        RATE_LIMIT_ATTEMPTS
            .with_label_values(&[bucket_name, "available"])
            .inc();

        Ok(RateLimit::Available((
            key_group_limit - count,
            key_group_limit,
        )))
    }
}

#[macro_export]
macro_rules! rate_limit {
    ($api_key:expr, $db:expr, $limit:tt, $group:expr) => {
        rate_limit!($api_key, $db, $limit, $group, 1)
    };

    ($api_key:expr, $db:expr, $limit:tt, $group:expr, $incr_by:expr) => {{
        let rate_limit =
            crate::update_rate_limit($db, $api_key.0.id, $api_key.0.$limit, $group, $incr_by)
                .await
                .map_err(crate::Error::from)?;

        match rate_limit {
            crate::RateLimit::Limited => {
                return Ok(crate::api::RateLimitedResponse::limited($group, 60))
            }
            crate::RateLimit::Available(count) => count,
        }
    }};
}

#[tracing::instrument(err, skip(pool, bkapi))]
async fn lookup_hashes(
    pool: &Pool,
    bkapi: &BKApiClient,
    hashes: &[i64],
    distance: u64,
) -> Result<Vec<HashLookupResult>, Error> {
    if distance > 10 {
        return Err(BadRequest::with_message(format!("distance too large: {}", distance)).into());
    }

    tracing::info!("looking up {} hashes", hashes.len());

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

    tracing::info!("found {} results in bkapi index", index_hashes.len());
    tracing::trace!(
        "bkapi matches: {:?}",
        index_hashes
            .iter()
            .map(|hash| hash.found_hash)
            .collect::<Vec<_>>()
    );

    let data = serde_json::to_value(index_hashes)?;

    let timer = IMAGE_QUERY_DURATION.start_timer();
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
    let seconds = timer.stop_and_record();

    tracing::info!(
        "found {} matches from database in {} seconds",
        results.len(),
        seconds
    );
    tracing::trace!("database matches: {:?}", results);

    Ok(results)
}

#[derive(Debug, Multipart)]
struct ImageSearchPayload {
    image: Upload,
}

#[tracing::instrument(skip(client, hash_input_endpoint, image))]
async fn hash_input(
    client: &reqwest::Client,
    hash_input_endpoint: &str,
    image: reqwest::Body,
) -> Result<i64, Error> {
    let part = reqwest::multipart::Part::stream(image);
    let form = reqwest::multipart::Form::new().part("image", part);

    tracing::info!("sending image for hashing");

    let timer = IMAGE_HASH_DURATION.start_timer();
    let resp = client
        .post(hash_input_endpoint)
        .multipart(form)
        .send()
        .await?;
    let seconds = timer.stop_and_record();

    tracing::info!("completed image hash in {} seconds", seconds);

    if resp.status() != StatusCode::OK {
        tracing::warn!("got wrong status code: {}", resp.status());
        return Err(BadRequest::with_message("invalid image").into());
    }

    let text = resp.text().await?;

    match text.parse() {
        Ok(hash) => {
            tracing::debug!("image had hash {}", hash);
            Ok(hash)
        }
        Err(_err) => {
            tracing::warn!("got invalid data: {}", text);
            Err(BadRequest::with_message("invalid image").into())
        }
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

trait ResponseRateLimitHeaders
where
    Self: Sized,
{
    fn inject_rate_limit_headers(self, name: &'static str, remaining: (i16, i16)) -> Self;
}

impl<T> ResponseRateLimitHeaders for poem_openapi::payload::Response<T> {
    fn inject_rate_limit_headers(self, name: &'static str, remaining: (i16, i16)) -> Self {
        self.header(&format!("x-rate-limit-total-{}", name), remaining.1)
            .header(&format!("x-rate-limit-remaining-{}", name), remaining.0)
    }
}

/// LimitsResponse
///
/// The allowed number of requests per minute for an API key.
#[derive(Object, Debug)]
struct LimitsResponse {
    /// The number of name lookups.
    name: i16,
    /// The number of hash lookups.
    image: i16,
    /// The number of image hashes.
    hash: i16,
}

#[OpenApi]
impl Api {
    /// Lookup images by hash
    ///
    /// Perform a lookup using up to 10 hashes.
    #[oai(path = "/hashes", method = "get")]
    async fn hashes(
        &self,
        pool: Data<&Pool>,
        bkapi: Data<&BKApiClient>,
        auth: ApiKeyAuthorization,
        hashes: Query<String>,
        distance: Query<Option<u64>>,
    ) -> poem::Result<Response<api::RateLimitedResponse<Vec<HashLookupResult>>>> {
        api::hashes(pool.0, bkapi.0, auth, hashes, distance).await
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
    ) -> poem::Result<Response<api::RateLimitedResponse<ImageSearchResult>>> {
        api::image(
            pool.0,
            bkapi.0,
            client.0,
            endpoints.0,
            auth,
            search_type,
            payload,
        )
        .await
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
    ) -> poem::Result<Response<api::RateLimitedResponse<ImageSearchResult>>> {
        api::url(pool.0, bkapi.0, client.0, endpoints.0, auth, url, distance).await
    }

    /// Lookup FurAffinity submission by File ID
    #[oai(path = "/furaffinity/file_id", method = "get")]
    async fn furaffinity_data(
        &self,
        pool: Data<&Pool>,
        auth: ApiKeyAuthorization,
        file_id: Query<i32>,
    ) -> poem::Result<Response<api::RateLimitedResponse<Vec<FurAffinityFile>>>> {
        api::furaffinity_data(pool.0, auth, file_id).await
    }

    /// Check API key limits
    ///
    /// Determine the number of allowed requests per minute for the current
    /// API token.
    #[oai(path = "/limits", method = "get")]
    async fn limits(&self, auth: ApiKeyAuthorization) -> Json<LimitsResponse> {
        Json(LimitsResponse {
            name: auth.0.name_limit,
            image: auth.0.image_limit,
            hash: auth.0.hash_limit,
        })
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
        api::known_service(pool.0, service, handle).await
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
        .at("/metrics", poem::endpoint::PrometheusExporter::new())
        .data(pool)
        .data(bkapi)
        .data(endpoints)
        .data(reqwest::Client::new())
        .with(poem::middleware::Tracing)
        .with(poem::middleware::OpenTelemetryMetrics::new())
        .with(poem::middleware::OpenTelemetryTracing::new(
            fuzzysearch_common::trace::get_tracer("fuzzysearch-api"),
        ))
        .with(cors);

    poem::Server::new(TcpListener::bind("0.0.0.0:8080"))
        .run(app)
        .await
        .unwrap();
}
