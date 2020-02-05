#![recursion_limit = "256"]

use std::str::FromStr;

mod filters;
mod handlers;
mod models;
mod types;
mod utils;

use warp::Filter;

fn configure_tracing() {
    use opentelemetry::{
        api::{KeyValue, Provider, Sampler},
        exporter::trace::jaeger,
        sdk::Config,
    };
    use tracing_subscriber::layer::SubscriberExt;

    let env = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    let exporter = jaeger::Exporter::builder()
        .with_collector_endpoint(std::env::var("JAEGER_COLLECTOR").unwrap().parse().unwrap())
        .with_process(jaeger::Process {
            service_name: "fuzzysearch",
            tags: vec![
                KeyValue::new("environment", env),
                KeyValue::new("version", env!("CARGO_PKG_VERSION")),
            ],
        })
        .init();

    let provider = opentelemetry::sdk::Provider::builder()
        .with_exporter(exporter)
        .with_config(Config {
            default_sampler: Sampler::Always,
            ..Default::default()
        })
        .build();

    let tracer = provider.get_tracer("api");

    let telem_layer = tracing_opentelemetry::OpentelemetryLayer::with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::Layer::default();

    let subscriber = tracing_subscriber::Registry::default()
        .with(telem_layer)
        .with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)
        .expect("Unable to set default tracing subscriber");
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    configure_tracing();

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

fn get_hasher() -> img_hash::Hasher<[u8; 8]> {
    use img_hash::{HashAlg::Gradient, HasherConfig};

    HasherConfig::with_bytes_type::<[u8; 8]>()
        .hash_alg(Gradient)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}
