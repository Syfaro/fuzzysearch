use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use fuzzysearch_common::faktory::FaktoryClient;

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

async fn load_frontpage(client: &reqwest::Client, api_key: &str) -> anyhow::Result<i32> {
    let resp: WeasylResponse<Vec<serde_json::Value>> = client
        .get("https://www.weasyl.com/api/submissions/frontpage")
        .header("X-Weasyl-API-Key", api_key)
        .send()
        .await?
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
        Err(_err) => return Ok((None, body)),
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

#[tracing::instrument(skip(pool, client, faktory, body, sub), fields(id = sub.id))]
async fn process_submission(
    pool: &sqlx::Pool<sqlx::Postgres>,
    client: &reqwest::Client,
    faktory: &FaktoryClient,
    body: serde_json::Value,
    sub: WeasylSubmission,
) -> anyhow::Result<()> {
    tracing::debug!("Processing submission");

    let data = client
        .get(&sub.media.submission.first().unwrap().url)
        .send()
        .await?
        .bytes()
        .await?;

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

    faktory
        .queue_webhook(fuzzysearch_common::types::WebHookData {
            site: fuzzysearch_common::types::Site::Weasyl,
            site_id: sub.id,
            artist: sub.owner_login.clone(),
            file_url: sub.media.submission.first().unwrap().url.clone(),
            file_sha256: Some(result.to_vec()),
            hash: num.map(|hash| hash.to_be_bytes()),
        })
        .await?;

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
    if matches!(std::env::var("LOG_FMT").as_deref(), Ok("json")) {
        tracing_subscriber::fmt::Subscriber::builder().json().init();
    } else {
        tracing_subscriber::fmt::init();
    }

    let api_key = std::env::var("WEASYL_APIKEY").unwrap();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();

    let faktory_dsn = std::env::var("FAKTORY_URL").expect("Missing FAKTORY_URL");
    let faktory = FaktoryClient::connect(faktory_dsn)
        .await
        .expect("Unable to connect to Faktory");

    loop {
        let min = sqlx::query!("SELECT max(id) id FROM weasyl")
            .fetch_one(&pool)
            .await
            .unwrap()
            .id
            .unwrap_or_default();

        let max = load_frontpage(&client, &api_key).await.unwrap();

        for id in (min + 1)..=max {
            let row: Option<_> = sqlx::query!("SELECT id FROM weasyl WHERE id = $1", id)
                .fetch_optional(&pool)
                .await
                .unwrap();
            if row.is_some() {
                continue;
            }

            match load_submission(&client, &api_key, id).await.unwrap() {
                (Some(sub), json) => process_submission(&pool, &client, &faktory, json, sub)
                    .await
                    .unwrap(),
                (None, body) => insert_null(&pool, body, id).await.unwrap(),
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(60 * 5)).await;
    }
}
