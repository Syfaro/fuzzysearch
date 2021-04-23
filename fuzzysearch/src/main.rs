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
    use opentelemetry::KeyValue;
    use tracing_subscriber::layer::SubscriberExt;

    let env = std::env::var("ENVIRONMENT");
    let env = if let Ok(env) = env.as_ref() {
        env.as_str()
    } else if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());

    let tracer = opentelemetry_jaeger::new_pipeline()
        .with_agent_endpoint(std::env::var("JAEGER_COLLECTOR").expect("Missing JAEGER_COLLECTOR"))
        .with_service_name("fuzzysearch")
        .with_tags(vec![
            KeyValue::new("environment", env.to_owned()),
            KeyValue::new("version", env!("CARGO_PKG_VERSION")),
        ])
        .install_batch(opentelemetry::runtime::Tokio)
        .unwrap();

    let trace = tracing_opentelemetry::layer().with_tracer(tracer);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    if matches!(std::env::var("LOG_FMT").as_deref(), Ok("json")) {
        let subscriber = tracing_subscriber::fmt::layer()
            .json()
            .with_timer(tracing_subscriber::fmt::time::ChronoUtc::rfc3339())
            .with_target(true);
        let subscriber = tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(trace)
            .with(subscriber);
        tracing::subscriber::set_global_default(subscriber).unwrap();
    } else {
        let subscriber = tracing_subscriber::fmt::layer();
        let subscriber = tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(trace)
            .with(subscriber);
        tracing::subscriber::set_global_default(subscriber).unwrap();
    }
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
        "SELECT hash_int hash FROM submission WHERE hash_int IS NOT NULL
        UNION
        SELECT hash FROM e621 WHERE hash IS NOT NULL
        UNION
        SELECT hash FROM tweet_media WHERE hash IS NOT NULL
        UNION
        SELECT hash FROM weasyl WHERE hash IS NOT NULL"
    )
    .fetch(conn);

    let mut count = 0;

    while let Some(row) = rows.try_next().await.expect("Unable to get row") {
        if let Some(hash) = row.hash {
            tree.add(Node::new(hash));
            count += 1;

            if count % 250_000 == 0 {
                tracing::debug!(count, "Made progress in loading tree rows");
            }
        }
    }

    tracing::info!(count, "Completed loading rows for tree");

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
