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
    fn distance(&self, a: &Node, b: &Node) -> u32 {
        hamming::distance_fast(&a.0, &b.0).unwrap() as u32
    }

    fn threshold_distance(&self, a: &Node, b: &Node, _threshold: u32) -> Option<u32> {
        Some(self.distance(a, b))
    }
}

#[derive(Clone)]
pub struct Endpoints {
    pub hash_input: String,
}

#[tokio::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch");
    fuzzysearch_common::trace::serve_metrics().await;

    ffmpeg_next::init().expect("Unable to initialize ffmpeg");

    let s = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let db_pool = sqlx::PgPool::connect(&s)
        .await
        .expect("Unable to create Postgres pool");

    let tree: Tree = Arc::new(RwLock::new(bk_tree::BKTree::new(Hamming)));

    let endpoints = Endpoints {
        hash_input: std::env::var("ENDPOINT_HASH_INPUT").expect("Missing ENDPOINT_HASH_INPUT"),
    };

    load_updates(db_pool.clone(), tree.clone()).await;

    let log = warp::log("fuzzysearch");
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["x-api-key"])
        .allow_methods(vec!["GET", "POST"]);

    let options = warp::options().map(|| "✓");

    let api = options.or(filters::search(db_pool, tree, endpoints));
    let routes = api
        .or(warp::path::end()
            .map(|| warp::redirect(warp::http::Uri::from_static("https://fuzzysearch.net"))))
        .with(log)
        .with(cors)
        .recover(handlers::handle_rejection);

    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
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
        UNION ALL
        SELECT hash FROM e621 WHERE hash IS NOT NULL
        UNION ALL
        SELECT hash FROM tweet_media WHERE hash IS NOT NULL
        UNION ALL
        SELECT hash FROM weasyl WHERE hash IS NOT NULL"
    )
    .fetch(conn);

    let mut count = 0;

    while let Some(row) = rows.try_next().await.expect("Unable to get row") {
        if let Some(hash) = row.hash {
            let node = Node::new(hash);
            if tree.find_exact(&node).is_none() {
                tree.add(node);
            }

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
