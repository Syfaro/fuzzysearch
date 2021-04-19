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
pub struct Node(pub [u8; 8]);

impl Node {
    pub fn new(hash: i64) -> Self {
        Self(hash.to_be_bytes())
    }

    pub fn query(hash: [u8; 8]) -> Self {
        Self(hash)
    }

    pub fn num(&self) -> i64 {
        i64::from_be_bytes(self.0)
    }
}

pub struct Hamming;

impl bk_tree::Metric<Node> for Hamming {
    fn distance(&self, a: &Node, b: &Node) -> u64 {
        hamming::distance_fast(&a.0, &b.0).unwrap()
    }
}

#[tokio::main]
async fn main() {
    configure_tracing();

    ffmpeg_next::init().expect("Unable to initialize ffmpeg");

    let s = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let db_pool = sqlx::PgPool::connect(&s)
        .await
        .expect("Unable to create Postgres pool");

    serve_metrics().await;

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
        .with_agent_endpoint(
            std::env::var("JAEGER_COLLECTOR")
                .expect("Missing JAEGER_COLLECTOR")
                .parse()
                .unwrap(),
        )
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

async fn metrics(
    _: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, std::convert::Infallible> {
    use hyper::{Body, Response};
    use prometheus::{Encoder, TextEncoder};

    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();

    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    Ok(Response::new(Body::from(buffer)))
}

async fn serve_metrics() {
    use hyper::{
        service::{make_service_fn, service_fn},
        Server,
    };
    use std::convert::Infallible;
    use std::net::SocketAddr;

    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(metrics)) });

    let addr: SocketAddr = std::env::var("METRICS_HOST")
        .expect("Missing METRICS_HOST")
        .parse()
        .expect("Invalid METRICS_HOST");

    let server = Server::bind(&addr).serve(make_svc);

    tokio::spawn(async move {
        server.await.expect("Metrics server error");
    });
}

#[derive(serde::Deserialize)]
struct HashRow {
    hash: i64,
}

async fn create_tree(conn: &Pool) -> bk_tree::BKTree<Node, Hamming> {
    use futures::TryStreamExt;

    let mut tree = bk_tree::BKTree::new(Hamming);

    let mut rows = sqlx::query!(
        "SELECT id, hash_int hash FROM submission WHERE hash_int IS NOT NULL
        UNION ALL
        SELECT id, hash FROM e621 WHERE hash IS NOT NULL
        UNION ALL
        SELECT tweet_id, hash FROM tweet_media WHERE hash IS NOT NULL
        UNION ALL
        SELECT id, hash FROM weasyl WHERE hash IS NOT NULL"
    )
    .fetch(conn);

    while let Some(row) = rows.try_next().await.expect("Unable to get row") {
        if let Some(hash) = row.hash {
            if tree.find_exact(&Node::new(hash)).is_some() {
                continue;
            }

            tree.add(Node::new(hash));
        }
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
                tracing::debug!(hash = payload.hash, "Adding new hash to tree");

                let lock = tree.read().await;
                if lock.find_exact(&Node::new(payload.hash)).is_some() {
                    continue;
                }
                drop(lock);

                let mut lock = tree.write().await;
                lock.add(Node(payload.hash.to_be_bytes()));
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
