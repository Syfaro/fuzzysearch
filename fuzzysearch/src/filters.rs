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
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("image")
        .and(warp::header::headers_cloned())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(warp::query::<ImageSearchOpts>())
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|headers, form, opts, pool, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_image", ?opts);
            span.set_parent(with_telem(headers));
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
        .and(warp::header::headers_cloned())
        .and(warp::get())
        .and(warp::query::<HashSearchOpts>())
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|headers, opts, db, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_hashes", ?opts);
            span.set_parent(with_telem(headers));
            span.in_scope(|| handlers::search_hashes(opts, db, tree, api_key).in_current_span())
        })
}

pub fn stream_search_image(
    db: Pool,
    tree: Tree,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("stream")
        .and(warp::header::headers_cloned())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(with_pool(db))
        .and(with_tree(tree))
        .and(with_api_key())
        .and_then(|headers, form, pool, tree, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("stream_search_image");
            span.set_parent(with_telem(headers));
            span.in_scope(|| handlers::stream_image(form, pool, tree, api_key).in_current_span())
        })
}

pub fn search_video(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("video")
        .and(warp::header::headers_cloned())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(|headers, form, db, api_key| {
            use tracing_opentelemetry::OpenTelemetrySpanExt;

            let span = tracing::info_span!("search_video");
            span.set_parent(with_telem(headers));
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

fn with_telem(headers: warp::http::HeaderMap) -> opentelemetry::Context {
    let remote_context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&opentelemetry_http::HeaderExtractor(&headers))
    });

    tracing::trace!(?remote_context, "Got remote context");

    remote_context
}
