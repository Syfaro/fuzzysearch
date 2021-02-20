use crate::types::*;
use crate::{handlers, Pool, Tree};
use std::convert::Infallible;
use tracing_futures::Instrument;
use warp::{Filter, Rejection, Reply};

pub fn search(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    search_image(db.clone(), tree.clone())
        .or(search_hashes(db.clone(), tree.clone()))
        .or(stream_search_image(db.clone(), tree.clone()))
        .or(search_file(db.clone()))
        .or(search_video(db.clone()))
        .or(check_handle(db.clone()))
        .or(search_image_by_url(db, tree))
}

pub fn search_file(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("file")
        .and(warp::header::optional::<String>("x-b3"))
        .and(warp::get())
        .and(warp::query::<FileSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(|b3, opts, db, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_file", ?opts);
            span.set_parent(&with_telem(b3));
            span.in_scope(|| handlers::search_file(opts, db, api_key).in_current_span())
        })
}

pub fn search_image(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("image")
        .and(warp::header::optional::<String>("x-b3"))
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(warp::query::<ImageSearchOpts>())
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|b3, form, opts, pool, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_image", ?opts);
            span.set_parent(&with_telem(b3));
            span.in_scope(|| {
                handlers::search_image(form, opts, pool, tree, api_key).in_current_span()
            })
        })
}

pub fn search_image_by_url(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("url")
        .and(warp::get())
        .and(warp::query::<UrlSearchOpts>())
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(handlers::search_image_by_url)
}

pub fn search_hashes(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("hashes")
        .and(warp::header::optional::<String>("x-b3"))
        .and(warp::get())
        .and(warp::query::<HashSearchOpts>())
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|b3, opts, db, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_hashes", ?opts);
            span.set_parent(&with_telem(b3));
            span.in_scope(|| handlers::search_hashes(opts, db, tree, api_key).in_current_span())
        })
}

pub fn stream_search_image(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("stream")
        .and(warp::header::optional::<String>("x-b3"))
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|b3, form, pool, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("stream_search_image");
            span.set_parent(&with_telem(b3));
            span.in_scope(|| handlers::stream_image(form, pool, tree, api_key).in_current_span())
        })
}

pub fn search_video(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("video")
        .and(warp::header::optional::<String>("x-b3"))
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(|b3, form, db, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_video");
            span.set_parent(&with_telem(b3));
            span.in_scope(|| handlers::search_video(form, db, api_key).in_current_span())
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

fn with_tree(tree: Tree) -> impl Filter<Extract = (Tree,), Error = Infallible> + Clone {
    warp::any().map(move || tree.clone())
}

fn with_telem(b3: Option<String>) -> opentelemetry::api::context::Context {
    use opentelemetry::api::{HttpTextFormat, TraceContextExt};

    let mut carrier = std::collections::HashMap::new();
    if let Some(b3) = b3 {
        // It took way too long to realize it's a case-sensitive comparison...
        // Looks like it should be fixed in the next release,
        // https://github.com/open-telemetry/opentelemetry-rust/pull/148
        carrier.insert("X-B3".to_string(), b3);
    }

    let propagator = opentelemetry::api::B3Propagator::new(true);
    let parent_context = propagator.extract(&carrier);
    tracing::trace!(
        "remote span context: {:?}",
        parent_context.remote_span_context()
    );

    parent_context
}
