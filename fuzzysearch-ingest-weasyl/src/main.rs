use std::time::Duration;

use prometheus::{register_counter, register_histogram, Counter, Histogram, HistogramOpts, Opts};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing_unwrap::{OptionExt, ResultExt};

use fuzzysearch_common::faktory::FaktoryClient;

lazy_static::lazy_static! {
    static ref INDEX_DURATION: Histogram = register_histogram!(HistogramOpts::new(
        "fuzzysearch_watcher_index_duration_seconds",
        "Duration to load an index of submissions"
    )
    .const_label("site", "weasyl"))
    .unwrap_or_log();
    static ref SUBMISSION_DURATION: Histogram = register_histogram!(HistogramOpts::new(
        "fuzzysearch_watcher_submission_duration_seconds",
        "Duration to load an index of submissions"
    )
    .const_label("site", "weasyl"))
    .unwrap_or_log();
    static ref SUBMISSION_MISSING: Counter = register_counter!(Opts::new(
        "fuzzysearch_watcher_submission_missing_total",
        "Number of submissions that were missing"
    )
    .const_label("site", "weasyl"))
    .unwrap_or_log();
}

#[derive(Debug, Serialize, Deserialize)]
struct WeasylMediaSubmission {
    #[serde(rename = "mediaid")]
    id: i32,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WeasylMedia {
    submission: Vec<WeasylMediaSubmission>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum WeasylSubmissionSubtype {
    Multimedia,
    Visual,
    Literary,
}

#[derive(Debug, Serialize, Deserialize)]
struct WeasylSubmission {
    #[serde(rename = "submitid")]
    id: i32,
    owner_login: String,
    media: WeasylMedia,
    subtype: WeasylSubmissionSubtype,
}

#[derive(Debug, Serialize, Deserialize)]
struct WeasylFrontpageSubmission {
    #[serde(rename = "submitid")]
    id: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct WeasylError {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum WeasylResponse<T> {
    Error { error: WeasylError },
    Response(T),
}

#[tracing::instrument(skip(client, api_key))]
async fn load_frontpage(client: &reqwest::Client, api_key: &str) -> anyhow::Result<i32> {
    let resp: WeasylResponse<Vec<serde_json::Value>> = client
        .get("https://www.weasyl.com/api/submissions/frontpage")
        .header("X-Weasyl-API-Key", api_key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let subs = match resp {
        WeasylResponse::Response(subs) => subs,
        WeasylResponse::Error {
            error: WeasylError { name },
        } => return Err(anyhow::anyhow!(name)),
    };

    let max = subs
        .into_iter()
        .filter_map(|sub| sub.get("submitid").and_then(|id| id.as_i64()))
        .max()
        .unwrap_or_default();

    Ok(max as i32)
}

#[tracing::instrument(skip(client, api_key))]
async fn load_submission(
    client: &reqwest::Client,
    api_key: &str,
    id: i32,
) -> anyhow::Result<(Option<WeasylSubmission>, serde_json::Value)> {
    tracing::debug!("Loading submission");

    let body: serde_json::Value = client
        .get(&format!(
            "https://www.weasyl.com/api/submissions/{}/view",
            id
        ))
        .header("X-Weasyl-API-Key", api_key)
        .send()
        .await?
        .json()
        .await?;

    let data: WeasylResponse<WeasylSubmission> = match serde_json::from_value(body.clone()) {
        Ok(data) => data,
        Err(err) => {
            tracing::error!("Unable to parse submission: {:?}", err);
            return Ok((None, body));
        }
    };

    let res = match data {
        WeasylResponse::Response(sub) if sub.subtype == WeasylSubmissionSubtype::Visual => {
            Some(sub)
        }
        WeasylResponse::Response(_sub) => None,
        WeasylResponse::Error {
            error: WeasylError { name },
        } if name == "submissionRecordMissing" => None,
        WeasylResponse::Error {
            error: WeasylError { name },
        } => return Err(anyhow::anyhow!(name)),
    };

    Ok((res, body))
}

#[tracing::instrument(skip(pool, client, faktory, body, sub, download_folder), fields(id = sub.id))]
async fn process_submission(
    pool: &sqlx::Pool<sqlx::Postgres>,
    client: &reqwest::Client,
    faktory: &FaktoryClient,
    body: serde_json::Value,
    sub: WeasylSubmission,
    download_folder: &Option<String>,
) -> anyhow::Result<()> {
    tracing::debug!("Processing submission");

    let data = client
        .get(&sub.media.submission.first().unwrap_or_log().url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();

    let num = if let Ok(image) = image::load_from_memory(&data) {
        let hasher = fuzzysearch_common::get_hasher();
        let hash = hasher.hash_image(&image);
        let mut bytes: [u8; 8] = [0; 8];
        bytes.copy_from_slice(hash.as_bytes());
        let num = i64::from_be_bytes(bytes);
        Some(num)
    } else {
        tracing::warn!("Unable to decode image");

        None
    };

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let result: [u8; 32] = hasher.finalize().into();

    if let Some(folder) = download_folder {
        if let Err(err) = fuzzysearch_common::download::write_bytes(folder, &result, &data).await {
            tracing::error!("Could not download image: {:?}", err);
        }
    }

    sqlx::query!(
        "INSERT INTO weasyl (id, hash, sha256, file_size, data) VALUES ($1, $2, $3, $4, $5)",
        sub.id,
        num,
        result.to_vec(),
        data.len() as i32,
        body
    )
    .execute(pool)
    .await?;

    tracing::info!("Completed submission");

    faktory
        .queue_webhook(fuzzysearch_common::faktory::WebHookData {
            site: fuzzysearch_common::types::Site::Weasyl,
            site_id: sub.id as i64,
            artist: sub.owner_login.clone(),
            file_url: sub.media.submission.first().unwrap_or_log().url.clone(),
            file_sha256: Some(result.to_vec()),
            hash: num.map(|hash| hash.to_be_bytes()),
        })
        .await?;

    Ok(())
}

#[tracing::instrument(skip(pool, body))]
async fn insert_null(
    pool: &sqlx::Pool<sqlx::Postgres>,
    body: serde_json::Value,
    id: i32,
) -> anyhow::Result<()> {
    tracing::debug!("Inserting null submission");

    sqlx::query!("INSERT INTO WEASYL (id, data) VALUES ($1, $2)", id, body)
        .execute(pool)
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch-ingest-weasyl");
    fuzzysearch_common::trace::serve_metrics().await;

    let api_key = std::env::var("WEASYL_APIKEY").unwrap_or_log();
    let user_agent = std::env::var("USER_AGENT").unwrap_or_log();

    let download_folder = std::env::var("DOWNLOAD_FOLDER").ok();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("DATABASE_URL").unwrap_or_log())
        .await
        .unwrap_or_log();

    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .unwrap_or_log();

    let faktory_dsn = std::env::var("FAKTORY_URL").expect_or_log("Missing FAKTORY_URL");
    let faktory = FaktoryClient::connect(faktory_dsn)
        .await
        .expect_or_log("Unable to connect to Faktory");

    loop {
        let min = sqlx::query!("SELECT max(id) id FROM weasyl")
            .fetch_one(&pool)
            .await
            .unwrap_or_log()
            .id
            .unwrap_or_default();

        let duration = INDEX_DURATION.start_timer();
        let max = load_frontpage(&client, &api_key).await.unwrap_or_log();
        duration.stop_and_record();

        tracing::info!(min, max, "Calculated range of submissions to check");

        tokio::time::sleep(Duration::from_secs(1)).await;

        for id in (min + 1)..=max {
            let row: Option<_> = sqlx::query!("SELECT id FROM weasyl WHERE id = $1", id)
                .fetch_optional(&pool)
                .await
                .unwrap_or_log();
            if row.is_some() {
                continue;
            }

            let duration = SUBMISSION_DURATION.start_timer();

            match load_submission(&client, &api_key, id).await.unwrap_or_log() {
                (Some(sub), json) => {
                    process_submission(&pool, &client, &faktory, json, sub, &download_folder)
                        .await
                        .unwrap_or_log();

                    duration.stop_and_record();
                }
                (None, body) => {
                    insert_null(&pool, body, id).await.unwrap_or_log();

                    SUBMISSION_MISSING.inc();
                    duration.stop_and_discard();
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        tokio::time::sleep(std::time::Duration::from_secs(60 * 5)).await;
    }
}
