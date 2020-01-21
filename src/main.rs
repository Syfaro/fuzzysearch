use std::str::FromStr;

mod filters;
mod handlers;
mod models;
mod types;
mod utils;

use warp::Filter;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let s = std::env::var("POSTGRES_DSN").expect("Missing POSTGRES_DSN");

    let manager = bb8_postgres::PostgresConnectionManager::new(
        tokio_postgres::Config::from_str(&s).expect("Invalid POSTGRES_DSN"),
        tokio_postgres::NoTls,
    );

    let db_pool = bb8::Pool::builder()
        .build(manager)
        .await
        .expect("Unable to build Postgres pool");

    let log = warp::log("fuzzysearch");
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["x-api-key"])
        .allow_methods(vec!["GET", "POST"]);

    let options = warp::options().map(|| "✓");

    let api = options.or(filters::search(db_pool));
    let routes = api
        .or(warp::path::end()
            .map(|| warp::redirect(warp::http::Uri::from_static("https://fuzzysearch.net"))))
        .with(log)
        .with(cors)
        .recover(handlers::handle_rejection);

    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
}

type Pool = bb8::Pool<bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>>;

fn get_hasher() -> img_hash::Hasher {
    use img_hash::{HashAlg::Gradient, HasherConfig};

    HasherConfig::new()
        .hash_alg(Gradient)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}
