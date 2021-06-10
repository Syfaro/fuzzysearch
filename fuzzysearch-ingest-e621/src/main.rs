use anyhow::Context;
use lazy_static::lazy_static;
use prometheus::{register_histogram, register_int_gauge, Histogram, IntGauge};
use sqlx::Connection;
use tracing_unwrap::ResultExt;

use fuzzysearch_common::faktory::FaktoryClient;

static USER_AGENT: &str = "e621-watcher / FuzzySearch Ingester / Syfaro <syfaro@huefox.com>";

lazy_static! {
    static ref SUBMISSION_BACKLOG: IntGauge = register_int_gauge!(
        "fuzzysearch_watcher_e621_submission_backlog",
        "Number of submissions behind the latest ID"
    )
    .unwrap_or_log();
    static ref INDEX_DURATION: Histogram = register_histogram!(
        "fuzzysearch_watcher_e621_index_duration",
        "Duration to load an index of submissions"
    )
    .unwrap_or_log();
    static ref SUBMISSION_DURATION: Histogram = register_histogram!(
        "fuzzysearch_watcher_e621_submission_duration",
        "Duration to ingest a submission"
    )
    .unwrap_or_log();
}

type Auth = (String, Option<String>);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fuzzysearch_common::trace::configure_tracing();
    fuzzysearch_common::trace::serve_metrics().await;

    let login = std::env::var("E621_LOGIN").expect_or_log("Missing E621_LOGIN");
    let api_key = std::env::var("E621_API_KEY").expect_or_log("Missing E621_API_KEY");
    let auth = (login, Some(api_key));

    let client = reqwest::ClientBuilder::default()
        .user_agent(USER_AGENT)
        .build()?;

    let mut conn = sqlx::PgConnection::connect(
        &std::env::var("DATABASE_URL").expect_or_log("Missing DATABASE_URL"),
    )
    .await?;

    let faktory_dsn = std::env::var("FAKTORY_URL").expect_or_log("Missing FAKTORY_URL");
    let faktory = FaktoryClient::connect(faktory_dsn)
        .await
        .expect_or_log("Unable to connect to Faktory");

    let max_id: i32 = sqlx::query!("SELECT max(id) max FROM e621")
        .fetch_one(&mut conn)
        .await?
        .max
        .unwrap_or(0);

    tracing::info!(max_id, "Found maximum ID in database");

    let mut now;
    let mut min_id = max_id;

    let mut latest_id: Option<i32> = None;

    loop {
        now = std::time::Instant::now();

        let lid = match latest_id {
            Some(latest_id) => latest_id,
            None => {
                let _hist = INDEX_DURATION.start_timer();
                let lid = get_latest_id(&client, &auth)
                    .await
                    .expect_or_log("Unable to get latest ID");
                drop(_hist);

                latest_id = Some(lid);

                lid
            }
        };

        let _hist = INDEX_DURATION.start_timer();
        let page = load_page(&client, &auth, min_id).await?;
        drop(_hist);

        let posts = get_page_posts(&page)?;
        let post_ids = get_post_ids(&posts);

        tracing::trace!(?post_ids, "Collected posts");

        min_id = match post_ids.iter().max() {
            Some(id) => *id,
            None => {
                tracing::info!("Found no new posts, sleeping");
                tokio::time::sleep(std::time::Duration::from_secs(60 * 5)).await;
                continue;
            }
        };

        SUBMISSION_BACKLOG.set((lid - min_id).into());

        let mut tx = conn.begin().await?;

        for post in posts {
            let _hist = SUBMISSION_DURATION.start_timer();
            insert_submission(&mut tx, &faktory, &client, post).await?;
            drop(_hist);

            SUBMISSION_BACKLOG.sub(1);
        }

        tx.commit().await?;

        let elapsed = now.elapsed().as_millis() as u64;
        if post_ids.contains(&lid) {
            tracing::info!(lid, "Page contained latest ID, sleeping");
            tokio::time::sleep(std::time::Duration::from_secs(60 * 5)).await;

            latest_id = None;
        } else if elapsed < 1000 {
            let delay = 1000 - elapsed;
            tracing::info!(delay, "Delaying before next request");
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
    }
}

fn get_page_posts(page: &serde_json::Value) -> anyhow::Result<&Vec<serde_json::Value>> {
    let page = match page {
        serde_json::Value::Object(ref obj) => obj,
        _ => return Err(anyhow::anyhow!("Top level object was not an object")),
    };

    let posts = page
        .get("posts")
        .context("Page did not contain posts object")?
        .as_array()
        .context("Posts was not an array")?;

    Ok(posts)
}

fn get_post_ids(posts: &[serde_json::Value]) -> Vec<i32> {
    let ids: Vec<i32> = posts
        .iter()
        .filter_map(|post| {
            let post = match post {
                serde_json::Value::Object(post) => post,
                _ => return None,
            };

            let id = match post.get("id")? {
                serde_json::Value::Number(num) => num.as_i64()? as i32,
                _ => return None,
            };

            Some(id)
        })
        .collect();

    ids
}

#[tracing::instrument(err, skip(client, auth))]
async fn get_latest_id(client: &reqwest::Client, auth: &Auth) -> anyhow::Result<i32> {
    tracing::debug!("Looking up current highest ID");

    let query = vec![("limit", "1")];

    let page: serde_json::Value = client
        .get("https://e621.net/posts.json")
        .query(&query)
        .basic_auth(&auth.0, auth.1.as_ref())
        .send()
        .await?
        .json()
        .await?;

    let posts = get_page_posts(&page)?;

    let id = get_post_ids(&posts)
        .into_iter()
        .max()
        .context("Page had no IDs")?;

    tracing::info!(id, "Found maximum ID");

    Ok(id)
}

#[tracing::instrument(err, skip(client, auth))]
async fn load_page(
    client: &reqwest::Client,
    auth: &Auth,
    after_id: i32,
) -> anyhow::Result<serde_json::Value> {
    tracing::debug!("Attempting to load page");

    let query = vec![
        ("limit", "320".to_string()),
        ("page", format!("a{}", after_id)),
    ];

    let body = client
        .get("https://e621.net/posts.json")
        .query(&query)
        .basic_auth(&auth.0, auth.1.as_ref())
        .send()
        .await?
        .json()
        .await?;

    Ok(body)
}

type ImageData = (Option<i64>, Option<String>, Option<Vec<u8>>);

#[tracing::instrument(err, skip(conn, faktory, client, post), fields(id))]
async fn insert_submission(
    conn: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    faktory: &FaktoryClient,
    client: &reqwest::Client,
    post: &serde_json::Value,
) -> anyhow::Result<()> {
    let id = post
        .get("id")
        .context("Post was missing ID")?
        .as_i64()
        .context("Post ID was not number")? as i32;

    tracing::Span::current().record("id", &id);
    tracing::debug!("Inserting submission");

    tracing::trace!(?post, "Evaluating post");

    let (hash, hash_error, sha256): ImageData = if let Some((url, ext)) = get_post_url_ext(&post) {
        let (hash, hash_error, sha256) =
            if url != "/images/deleted-preview.png" && (ext == "jpg" || ext == "png") {
                load_image(&client, &url).await?
            } else {
                tracing::debug!("Ignoring post as it is deleted or not a supported image format");

                (None, None, None)
            };

        let artist = post
            .as_object()
            .and_then(|post| post.get("tags"))
            .and_then(|tags| tags.get("artist"))
            .and_then(|artist| artist.as_array())
            .map(|artists| {
                artists
                    .iter()
                    .filter_map(|artist| artist.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        faktory
            .queue_webhook(fuzzysearch_common::types::WebHookData {
                site: fuzzysearch_common::types::Site::E621,
                site_id: id,
                artist,
                file_url: url.to_owned(),
                file_sha256: sha256.clone(),
                hash: hash.map(|hash| hash.to_be_bytes()),
            })
            .await?;

        (hash, hash_error, sha256)
    } else {
        tracing::warn!("Post had missing URL or extension");

        (None, None, None)
    };

    sqlx::query!(
        "INSERT INTO e621
            (id, data, hash, hash_error, sha256) VALUES
            ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
                data = EXCLUDED.data,
                hash = EXCLUDED.hash,
                hash_error = EXCLUDED.hash_error,
                sha256 = EXCLUDED.sha256",
        id,
        post,
        hash,
        hash_error,
        sha256
    )
    .execute(conn)
    .await?;

    tracing::info!("Completed submission");

    Ok(())
}

fn get_post_url_ext(post: &serde_json::Value) -> Option<(&str, &str)> {
    let file = post.as_object()?.get("file")?.as_object()?;

    let url = file.get("url")?.as_str()?;
    let ext = file.get("ext")?.as_str()?;

    Some((url, ext))
}

#[tracing::instrument(err, skip(client))]
async fn load_image(client: &reqwest::Client, url: &str) -> anyhow::Result<ImageData> {
    use sha2::{Digest, Sha256};
    use std::convert::TryInto;

    let bytes = client.get(url).send().await?.bytes().await?;

    tracing::trace!(len = bytes.len(), "Got submission image bytes");

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize().to_vec();

    tracing::trace!(?result, "Calculated image SHA256");

    let hasher = fuzzysearch_common::get_hasher();
    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(err) => {
            tracing::error!(?err, "Unable to open image");
            return Ok((None, Some(err.to_string()), Some(result)));
        }
    };

    tracing::trace!("Opened image successfully");

    let hash = hasher.hash_image(&img);
    let hash: [u8; 8] = hash.as_bytes().try_into()?;
    let hash = i64::from_be_bytes(hash);

    tracing::trace!(?hash, "Calculated image hash");

    Ok((Some(hash), None, Some(result)))
}
