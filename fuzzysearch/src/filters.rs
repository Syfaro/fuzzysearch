use crate::{handlers, Pool};
use crate::{types::*, Endpoints};
use std::convert::Infallible;
use tracing_futures::Instrument;
use warp::{Filter, Rejection, Reply};

pub fn search(
    db: Pool,
    bkapi: bkapi_client::BKApiClient,
    endpoints: Endpoints,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    search_image(db.clone(), bkapi.clone(), endpoints)
        .or(search_hashes(db.clone(), bkapi.clone()))
        .or(search_file(db.clone()))
        .or(check_handle(db.clone()))
        .or(search_image_by_url(db, bkapi))
}

pub fn search_file(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("file")
        .and(warp::header::headers_cloned())
        .and(warp::get())
        .and(warp::query::<FileSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(|headers, opts, db, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_file", ?opts);
            span.set_parent(with_telem(headers));
            span.in_scope(|| handlers::search_file(opts, db, api_key).in_current_span())
        })
}

pub fn search_image(
    db: Pool,
    bkapi: bkapi_client::BKApiClient,
    endpoints: Endpoints,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("image")
        .and(warp::header::headers_cloned())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(warp::query::<ImageSearchOpts>())
        .and(with_pool(db))
        .and(with_bkapi(bkapi))
        .and(with_api_key())
        .and(with_endpoints(endpoints))
        .and_then(|headers, form, opts, pool, bkapi, api_key, endpoints| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_image", ?opts);
            span.set_parent(with_telem(headers));
            span.in_scope(|| {
                handlers::search_image(form, opts, pool, bkapi, api_key, endpoints)
                    .in_current_span()
            })
        })
}

pub fn search_image_by_url(
    db: Pool,
    bkapi: bkapi_client::BKApiClient,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("url")
        .and(warp::get())
        .and(warp::query::<UrlSearchOpts>())
        .and(with_pool(db))
        .and(with_bkapi(bkapi))
        .and(with_api_key())
        .and_then(handlers::search_image_by_url)
}

pub fn search_hashes(
    db: Pool,
    bkapi: bkapi_client::BKApiClient,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("hashes")
        .and(warp::header::headers_cloned())
        .and(warp::get())
        .and(warp::query::<HashSearchOpts>())
        .and(with_pool(db))
        .and(with_bkapi(bkapi))
        .and(with_api_key())
        .and_then(|headers, opts, db, bkapi, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_hashes", ?opts);
            span.set_parent(with_telem(headers));
            span.in_scope(|| handlers::search_hashes(opts, db, bkapi, api_key).in_current_span())
        })
}

pub fn check_handle(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("handle")
        .and(warp::get())
        .and(warp::query::<HandleOpts>())
        .and(with_pool(db))
        .and_then(handlers::check_handle)
}

fn with_api_key() -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
    warp::header::<String>("x-api-key")
}

fn with_pool(db: Pool) -> impl Filter<Extract = (Pool,), Error = Infallible> + Clone {
    warp::any().map(move || db.clone())
}

fn with_bkapi(
    bkapi: bkapi_client::BKApiClient,
) -> impl Filter<Extract = (bkapi_client::BKApiClient,), Error = Infallible> + Clone {
    warp::any().map(move || bkapi.clone())
}

fn with_endpoints(
    endpoints: Endpoints,
) -> impl Filter<Extract = (Endpoints,), Error = Infallible> + Clone {
    warp::any().map(move || endpoints.clone())
}

fn with_telem(headers: warp::http::HeaderMap) -> opentelemetry::Context {
    let remote_context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&opentelemetry_http::HeaderExtractor(&headers))
    });

    tracing::trace!(?remote_context, "Got remote context");

    remote_context
}
