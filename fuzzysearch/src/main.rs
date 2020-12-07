#![recursion_limit = "256"]

use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

mod filters;
mod handlers;
mod models;
mod types;
mod utils;

use warp::Filter;

fn configure_tracing() {
    use opentelemetry::{
        api::{KeyValue, Provider},
        sdk::{Config, Sampler},
    };
    use tracing_subscriber::{layer::SubscriberExt, prelude::*};

    let env = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    let fmt_layer = tracing_subscriber::fmt::layer();
    let filter_layer = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
        .unwrap();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .finish();
    let registry = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer);

    let exporter = opentelemetry_jaeger::Exporter::builder()
        .with_agent_endpoint(std::env::var("JAEGER_COLLECTOR").unwrap().parse().unwrap())
        .with_process(opentelemetry_jaeger::Process {
            service_name: "fuzzysearch".to_string(),
            tags: vec![
                KeyValue::new("environment", env),
                KeyValue::new("version", env!("CARGO_PKG_VERSION")),
            ],
        })
        .init()
        .expect("unable to create jaeger exporter");

    let provider = opentelemetry::sdk::Provider::builder()
        .with_simple_exporter(exporter)
        .with_config(Config {
            default_sampler: Box::new(Sampler::Always),
            ..Default::default()
        })
        .build();

    opentelemetry::global::set_provider(provider);

    let tracer = opentelemetry::global::trace_provider().get_tracer("fuzzysearch");
    let telem_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let registry = registry.with(telem_layer);

    registry.init();
}

#[derive(Debug)]
pub struct Node {
    id: i32,
    hash: [u8; 8],
}

impl Node {
    pub fn query(hash: [u8; 8]) -> Self {
        Self { id: -1, hash }
    }
}

type Tree = Arc<RwLock<bk_tree::BKTree<Node, Hamming>>>;

pub struct Hamming;

impl bk_tree::Metric<Node> for Hamming {
    fn distance(&self, a: &Node, b: &Node) -> u64 {
        hamming::distance_fast(&a.hash, &b.hash).unwrap()
    }
}

#[tokio::main]
async fn main() {
    ffmpeg_next::init().expect("Unable to initialize ffmpeg");

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

    let tree: Tree = Arc::new(RwLock::new(bk_tree::BKTree::new(Hamming)));

    let mut max_id = 0;

    let conn = db_pool.get().await.unwrap();
    let mut lock = tree.write().await;
    conn.query("SELECT id, hash FROM hashes", &[])
        .await
        .unwrap()
        .into_iter()
        .for_each(|row| {
            let id: i32 = row.get(0);
            let hash: i64 = row.get(1);
            let bytes = hash.to_be_bytes();

            if id > max_id {
                max_id = id;
            }

            lock.add(Node { id, hash: bytes });
        });
    drop(lock);
    drop(conn);

    let tree_clone = tree.clone();
    let pool_clone = db_pool.clone();
    tokio::spawn(async move {
        use futures_util::StreamExt;

        let max_id = std::sync::atomic::AtomicI32::new(max_id);
        let tree = tree_clone;
        let pool = pool_clone;

        let order = std::sync::atomic::Ordering::SeqCst;

        let interval = tokio::time::interval(std::time::Duration::from_secs(30));

        interval
            .for_each(|_| async {
                tracing::debug!("Refreshing hashes");

                let conn = pool.get().await.unwrap();
                let mut lock = tree.write().await;
                let id = max_id.load(order);

                let mut count = 0;

                conn.query("SELECT id, hash FROM hashes WHERE hashes.id > $1", &[&id])
                    .await
                    .unwrap()
                    .into_iter()
                    .for_each(|row| {
                        let id: i32 = row.get(0);
                        let hash: i64 = row.get(1);
                        let bytes = hash.to_be_bytes();

                        if id > max_id.load(order) {
                            max_id.store(id, order);
                        }

                        lock.add(Node { id, hash: bytes });

                        count += 1;
                    });

                tracing::trace!("Added {} new hashes", count);
            })
            .await;
    });

    let log = warp::log("fuzzysearch");
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["x-api-key"])
        .allow_methods(vec!["GET", "POST"]);

    let options = warp::options().map(|| "✓");

    let api = options.or(filters::search(db_pool, tree));
    let routes = api
        .or(warp::path::end()
            .map(|| warp::redirect(warp::http::Uri::from_static("https://fuzzysearch.net"))))
        .with(log)
        .with(cors)
        .recover(handlers::handle_rejection);

    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
}

type Pool = bb8::Pool<bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>>;
