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

type Span = Option<opentelemetry::global::BoxedSpan>;

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

    opentelemetry::global::set_provider(provider);

    let tracer = opentelemetry::global::trace_provider().get_tracer("api");

    let telem_layer = tracing_opentelemetry::OpentelemetryLayer::with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::Layer::default();

    let subscriber = tracing_subscriber::Registry::default()
        .with(telem_layer)
        .with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)
        .expect("Unable to set default tracing subscriber");
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

fn get_hasher() -> img_hash::Hasher<[u8; 8]> {
    use img_hash::{HashAlg::Gradient, HasherConfig};

    HasherConfig::with_bytes_type::<[u8; 8]>()
        .hash_alg(Gradient)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}
