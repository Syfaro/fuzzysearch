#![recursion_limit = "256"]

use warp::Filter;

mod filters;
mod handlers;
mod models;
mod types;
mod utils;

type Pool = sqlx::PgPool;

#[derive(Clone)]
pub struct Endpoints {
    pub hash_input: String,
    pub bkapi: String,
}

#[tokio::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch");
    fuzzysearch_common::trace::serve_metrics().await;

    let s = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let db_pool = sqlx::PgPool::connect(&s)
        .await
        .expect("Unable to create Postgres pool");

    let endpoints = Endpoints {
        hash_input: std::env::var("ENDPOINT_HASH_INPUT").expect("Missing ENDPOINT_HASH_INPUT"),
        bkapi: std::env::var("ENDPOINT_BKAPI").expect("Missing ENDPOINT_BKAPI"),
    };

    let bkapi = bkapi_client::BKApiClient::new(&endpoints.bkapi);

    let log = warp::log("fuzzysearch");
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["x-api-key"])
        .allow_methods(vec!["GET", "POST"]);

    let options = warp::options().map(|| "✓");

    let api = options.or(filters::search(db_pool, bkapi, endpoints));
    let routes = api
        .or(warp::path::end()
            .map(|| warp::redirect(warp::http::Uri::from_static("https://fuzzysearch.net"))))
        .with(log)
        .with(cors)
        .recover(handlers::handle_rejection);

    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
}
