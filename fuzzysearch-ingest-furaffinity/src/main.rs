use lazy_static::lazy_static;
use tokio_postgres::Client;
use tracing_unwrap::{OptionExt, ResultExt};

use fuzzysearch_common::faktory::FaktoryClient;

lazy_static! {
    static ref SUBMISSION_DURATION: prometheus::Histogram = prometheus::register_histogram!(
        "fuzzysearch_watcher_fa_processing_seconds",
        "Duration to process a submission"
    )
    .unwrap_or_log();
    static ref USERS_ONLINE: prometheus::IntGaugeVec = prometheus::register_int_gauge_vec!(
        "fuzzysearch_watcher_fa_users_online_count",
        "Number of users online for each category",
        &["group"]
    )
    .unwrap_or_log();
}

async fn lookup_tag(client: &Client, tag: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM tag WHERE name = $1", &[&tag])
        .await
        .unwrap_or_log()
        .into_iter()
        .next()
    {
        return row.get("id");
    }

    client
        .query("INSERT INTO tag (name) VALUES ($1) RETURNING id", &[&tag])
        .await
        .unwrap_or_log()
        .into_iter()
        .next()
        .unwrap_or_log()
        .get("id")
}

async fn lookup_artist(client: &Client, artist: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM artist WHERE name = $1", &[&artist])
        .await
        .unwrap_or_log()
        .into_iter()
        .next()
    {
        return row.get("id");
    }

    client
        .query(
            "INSERT INTO artist (name) VALUES ($1) RETURNING id",
            &[&artist],
        )
        .await
        .unwrap_or_log()
        .into_iter()
        .next()
        .unwrap_or_log()
        .get("id")
}

async fn has_submission(client: &Client, id: i32) -> bool {
    client
        .query("SELECT id FROM submission WHERE id = $1", &[&id])
        .await
        .unwrap_or_log()
        .into_iter()
        .next()
        .is_some()
}

async fn ids_to_check(client: &Client, max: i32) -> Vec<i32> {
    let rows = client.query("SELECT sid FROM generate_series((SELECT max(id) FROM submission), $1::int) sid WHERE sid NOT IN (SELECT id FROM submission where id = sid)", &[&max]).await.unwrap_or_log();

    rows.iter().map(|row| row.get("sid")).collect()
}

async fn insert_submission(
    client: &Client,
    sub: &furaffinity_rs::Submission,
) -> Result<(), tokio_postgres::Error> {
    let artist_id = lookup_artist(client, &sub.artist).await;
    let mut tag_ids = Vec::with_capacity(sub.tags.len());
    for tag in &sub.tags {
        tag_ids.push(lookup_tag(client, tag).await);
    }

    let hash = sub.hash.clone();
    let url = sub.content.url();

    let size = sub.file_size.map(|size| size as i32);

    client.execute("INSERT INTO submission (id, artist_id, url, filename, hash, rating, posted_at, description, hash_int, file_id, file_size, file_sha256) VALUES ($1, $2, $3, $4, decode($5, 'base64'), $6, $7, $8, $9, CASE WHEN isnumeric(split_part($4, '.', 1)) THEN split_part($4, '.', 1)::int ELSE null END, $10, $11)", &[
        &sub.id, &artist_id, &url, &sub.filename, &hash, &sub.rating.serialize(), &sub.posted_at, &sub.description, &sub.hash_num, &size, &sub.file_sha256,
    ]).await?;

    let stmt = client
        .prepare("INSERT INTO tag_to_post (tag_id, post_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .await?;

    for tag_id in tag_ids {
        client.execute(&stmt, &[&tag_id, &sub.id]).await?;
    }

    Ok(())
}

async fn insert_null_submission(client: &Client, id: i32) -> Result<u64, tokio_postgres::Error> {
    client
        .execute("INSERT INTO SUBMISSION (id) VALUES ($1)", &[&id])
        .await
}

struct RetryHandler {
    max_attempts: usize,
}

impl RetryHandler {
    fn new(max_attempts: usize) -> Self {
        Self { max_attempts }
    }
}

impl futures_retry::ErrorHandler<furaffinity_rs::Error> for RetryHandler {
    type OutError = furaffinity_rs::Error;

    #[tracing::instrument(skip(self), fields(max_attempts = self.max_attempts))]
    fn handle(
        &mut self,
        attempt: usize,
        err: furaffinity_rs::Error,
    ) -> futures_retry::RetryPolicy<Self::OutError> {
        tracing::warn!("Attempt failed");

        if attempt >= self.max_attempts {
            tracing::error!("All attempts have been used");
            return futures_retry::RetryPolicy::ForwardError(err);
        }

        if !err.retry {
            tracing::error!("Error did not ask for retry");
            return futures_retry::RetryPolicy::ForwardError(err);
        }

        futures_retry::RetryPolicy::WaitRetry(std::time::Duration::from_secs(1 + attempt as u64))
    }
}

#[tracing::instrument(skip(client, fa, faktory))]
async fn process_submission(
    client: &Client,
    fa: &furaffinity_rs::FurAffinity,
    faktory: &FaktoryClient,
    id: i32,
) {
    if has_submission(client, id).await {
        return;
    }

    tracing::info!("Loading submission");

    let _timer = SUBMISSION_DURATION.start_timer();

    let sub = futures_retry::FutureRetry::new(|| fa.get_submission(id), RetryHandler::new(3))
        .await
        .map(|(sub, _attempts)| sub)
        .map_err(|(err, _attempts)| err);

    let sub = match sub {
        Ok(sub) => sub,
        Err(err) => {
            tracing::error!("Failed to load submission: {:?}", err);
            _timer.stop_and_discard();
            insert_null_submission(client, id).await.unwrap_or_log();
            return;
        }
    };

    let sub = match sub {
        Some(sub) => sub,
        None => {
            tracing::warn!("Submission did not exist");
            _timer.stop_and_discard();
            insert_null_submission(client, id).await.unwrap_or_log();
            return;
        }
    };

    let image =
        futures_retry::FutureRetry::new(|| fa.calc_image_hash(sub.clone()), RetryHandler::new(3))
            .await
            .map(|(sub, _attempt)| sub)
            .map_err(|(err, _attempt)| err);

    let sub = match image {
        Ok(sub) => sub,
        Err(err) => {
            tracing::error!("Unable to hash submission image: {:?}", err);
            sub
        }
    };

    _timer.stop_and_record();

    if let Err(err) = faktory
        .queue_webhook(fuzzysearch_common::types::WebHookData {
            site: fuzzysearch_common::types::Site::FurAffinity,
            site_id: sub.id,
            artist: sub.artist.clone(),
            file_url: sub.content.url().clone(),
            file_sha256: sub.file_sha256.clone(),
            hash: sub.hash_num.map(|hash| hash.to_be_bytes()),
        })
        .await
    {
        tracing::error!("Unable to queue webhook: {:?}", err);
    }

    insert_submission(client, &sub).await.unwrap_or_log();
}

#[tokio::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch-ingest-furaffinity");
    fuzzysearch_common::trace::serve_metrics().await;

    let (cookie_a, cookie_b) = (
        std::env::var("FA_A").expect_or_log("Missing FA_A"),
        std::env::var("FA_B").expect_or_log("Missing FA_B"),
    );

    let user_agent = std::env::var("USER_AGENT").expect_or_log("Missing USER_AGENT");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_log();

    let fa = furaffinity_rs::FurAffinity::new(cookie_a, cookie_b, user_agent, Some(client));

    let dsn = std::env::var("POSTGRES_DSN").expect_or_log("Missing POSTGRES_DSN");

    let (client, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .unwrap_or_log();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            panic!("PostgreSQL connection error: {:?}", e);
        }
    });

    let faktory_dsn = std::env::var("FAKTORY_URL").expect_or_log("Missing FAKTORY_URL");
    let faktory = FaktoryClient::connect(faktory_dsn)
        .await
        .expect_or_log("Unable to connect to Faktory");

    tracing::info!("Started");

    loop {
        tracing::debug!("Fetching latest ID... ");
        let (latest_id, online) = fa
            .latest_id()
            .await
            .expect_or_log("Unable to get latest id");
        tracing::info!(latest_id = latest_id, "Got latest ID");

        tracing::debug!(?online, "Got updated users online");
        USERS_ONLINE
            .with_label_values(&["guest"])
            .set(online.guests as i64);
        USERS_ONLINE
            .with_label_values(&["registered"])
            .set(online.registered as i64);
        USERS_ONLINE
            .with_label_values(&["other"])
            .set(online.other as i64);

        for id in ids_to_check(&client, latest_id).await {
            process_submission(&client, &fa, &faktory, id).await;
        }

        tracing::info!("Completed fetch, waiting a minute before loading more");
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
