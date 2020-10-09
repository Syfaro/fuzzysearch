use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    let resp: WeasylResponse<Vec<WeasylFrontpageSubmission>> = client
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

    let max = subs.into_iter().max_by_key(|sub| sub.id);

    Ok(max.map(|sub| sub.id).unwrap_or_default())
}

async fn load_submission(
    client: &reqwest::Client,
    api_key: &str,
    id: i32,
) -> anyhow::Result<(Option<WeasylSubmission>, serde_json::Value)> {
    println!("Loading submission {}", id);

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

async fn process_submission(
    pool: &sqlx::Pool<sqlx::Postgres>,
    client: &reqwest::Client,
    body: serde_json::Value,
    sub: WeasylSubmission,
) -> anyhow::Result<()> {
    println!("Processing submission {}", sub.id);

    let data = client
        .get(&sub.media.submission.first().unwrap().url)
        .send()
        .await?
        .bytes()
        .await?;

    let num = if let Ok(image) = image::load_from_memory(&data) {
        let hasher = img_hash::HasherConfig::with_bytes_type::<[u8; 8]>()
            .hash_alg(img_hash::HashAlg::Gradient)
            .hash_size(8, 8)
            .preproc_dct()
            .to_hasher();
        let hash = hasher.hash_image(&image);
        let mut bytes: [u8; 8] = [0; 8];
        bytes.copy_from_slice(hash.as_bytes());
        let num = i64::from_be_bytes(bytes);
        Some(num)
    } else {
        println!("Unable to decode image on submission {}", sub.id);

        None
    };

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let result: [u8; 32] = hasher.finalize().into();

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

async fn insert_null(
    pool: &sqlx::Pool<sqlx::Postgres>,
    body: serde_json::Value,
    id: i32,
) -> anyhow::Result<()> {
    println!("Inserting null for submission {}", id);

    sqlx::query!("INSERT INTO WEASYL (id, data) VALUES ($1, $2)", id, body)
        .execute(pool)
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() {
    let api_key = std::env::var("WEASYL_APIKEY").unwrap();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();

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
            (Some(sub), json) => process_submission(&pool, &client, json, sub).await.unwrap(),
            (None, body) => insert_null(&pool, body, id).await.unwrap(),
        }
    }
}
