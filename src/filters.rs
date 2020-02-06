use crate::types::*;
use crate::{handlers, Pool};
use std::convert::Infallible;
use warp::{Filter, Rejection, Reply};

pub fn search(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    search_image(db.clone())
        .or(search_hashes(db.clone()))
        .or(stream_search_image(db.clone()))
        .or(search_file(db))
}

pub fn search_file(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("file")
        .and(with_telem())
        .and(warp::get())
        .and(warp::query::<FileSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::search_file)
}

pub fn search_image(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("image")
        .and(with_telem())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(warp::query::<ImageSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::search_image)
}

pub fn search_hashes(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("hashes")
        .and(with_telem())
        .and(warp::get())
        .and(warp::query::<HashSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::search_hashes)
}

pub fn stream_search_image(
    db: Pool,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("stream")
        .and(with_telem())
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::stream_image)
}

fn with_api_key() -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
    warp::header::<String>("x-api-key")
}

fn with_pool(db: Pool) -> impl Filter<Extract = (Pool,), Error = Infallible> + Clone {
    warp::any().map(move || db.clone())
}

fn with_telem() -> impl Filter<Extract = (crate::Span,), Error = Rejection> + Clone {
    warp::any()
        .and(warp::header::optional("traceparent"))
        .map(|traceparent: Option<String>| {
            use opentelemetry::api::trace::{provider::Provider, tracer::Tracer, propagator::HttpTextFormat};

            let mut headers = std::collections::HashMap::new();
            headers.insert("Traceparent", traceparent.unwrap_or_else(String::new));

            let propagator = opentelemetry::api::distributed_context::http_trace_context_propagator::HTTPTraceContextPropagator::new();
            let context = propagator.extract(&headers);

            tracing::trace!("got context from request: {:?}", context);

            let span = if context.is_valid() {
                let tracer = opentelemetry::global::trace_provider().get_tracer("api");
                let span = tracer.start("context", Some(context));
                tracer.mark_span_as_active(&span);

                Some(span)
            } else {
                None
            };

            span
        })
}
