use crate::types::*;
use crate::{handlers, Pool};
use std::convert::Infallible;
use warp::{Filter, Rejection, Reply};

pub fn search(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    search_file(db.clone())
        .or(search_image(db.clone()))
        .or(search_hashes(db.clone()))
        .or(stream_search_image(db))
}

pub fn search_file(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("file")
        .and(warp::get())
        .and(warp::query::<FileSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::search_file)
}

pub fn search_image(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("image")
        .and(warp::post())
        .and(warp::multipart::form().max_length(1024 * 1024 * 10))
        .and(warp::query::<ImageSearchOpts>())
        .and(with_pool(db))
        .and(with_api_key())
        .and_then(handlers::search_image)
}

pub fn search_hashes(db: Pool) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    warp::path("hashes")
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
