#![recursion_limit = "256"]

use std::sync::Arc;
use tokio::sync::RwLock;
use warp::Filter;

mod filters;
mod handlers;
mod models;
mod types;
mod utils;

type Tree = Arc<RwLock<bk_tree::BKTree<Node, Hamming>>>;
type Pool = sqlx::PgPool;

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

pub struct Hamming;

impl bk_tree::Metric<Node> for Hamming {
    fn distance(&self, a: &Node, b: &Node) -> u64 {
        hamming::distance_fast(&a.hash, &b.hash).unwrap()
    }
}

#[tokio::main]
async fn main() {
    configure_tracing();

    let s = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let db_pool = sqlx::PgPool::connect(&s)
        .await
        .expect("Unable to create Postgres pool");

    let tree: Tree = Arc::new(RwLock::new(bk_tree::BKTree::new(Hamming)));

    load_updates(db_pool.clone(), tree.clone()).await;

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

#[derive(serde::Deserialize)]
struct HashRow {
    id: i32,
    hash: i64,
}

async fn create_tree(conn: &Pool) -> bk_tree::BKTree<Node, Hamming> {
    use futures::TryStreamExt;

    let mut tree = bk_tree::BKTree::new(Hamming);

    let mut rows = sqlx::query_as!(HashRow, "SELECT id, hash FROM hashes").fetch(conn);

    while let Some(row) = rows.try_next().await.expect("Unable to get row") {
        tree.add(Node {
            id: row.id,
            hash: row.hash.to_be_bytes(),
        })
    }

    tree
}

async fn load_updates(conn: Pool, tree: Tree) {
    let mut listener = sqlx::postgres::PgListener::connect_with(&conn)
        .await
        .unwrap();
    listener.listen("fuzzysearch_hash_added").await.unwrap();

    let new_tree = create_tree(&conn).await;
    let mut lock = tree.write().await;
    *lock = new_tree;
    drop(lock);

    tokio::spawn(async move {
        loop {
            while let Some(notification) = listener
                .try_recv()
                .await
                .expect("Unable to recv notification")
            {
                let payload: HashRow = serde_json::from_str(notification.payload()).unwrap();
                tracing::debug!(id = payload.id, "Adding new hash to tree");

                let mut lock = tree.write().await;
                lock.add(Node {
                    id: payload.id,
                    hash: payload.hash.to_be_bytes(),
                });
                drop(lock);
            }

            tracing::error!("Lost connection to Postgres, recreating tree");
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let new_tree = create_tree(&conn).await;
            let mut lock = tree.write().await;
            *lock = new_tree;
            drop(lock);
            tracing::info!("Replaced tree");
        }
    });
}

fn get_hasher() -> img_hash::Hasher<[u8; 8]> {
    use img_hash::{HashAlg::Gradient, HasherConfig};

    HasherConfig::with_bytes_type::<[u8; 8]>()
        .hash_alg(Gradient)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}
