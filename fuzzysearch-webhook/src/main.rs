use r2d2_postgres::{postgres::NoTls, PostgresConnectionManager};
use thiserror::Error;
use tracing_unwrap::ResultExt;

static APP_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " - ",
    env!("CARGO_PKG_AUTHORS")
);

#[derive(Error, Debug)]
pub enum WebhookError {
    #[error("invalid data")]
    Serde(#[from] serde_json::Error),
    #[error("missing data")]
    MissingData,
    #[error("database pool issue")]
    Pool(#[from] r2d2_postgres::postgres::Error),
    #[error("database error")]
    Database(#[from] r2d2::Error),
    #[error("network error")]
    Network(#[from] reqwest::Error),
}

fn main() {
    fuzzysearch_common::init_logger();

    tracing::info!("Starting...");

    let dsn = std::env::var("POSTGRES_DSN").unwrap_or_log();
    let manager = PostgresConnectionManager::new(dsn.parse().unwrap_or_log(), NoTls);
    let pool = r2d2::Pool::new(manager).unwrap_or_log();

    let client = reqwest::blocking::ClientBuilder::default()
        .user_agent(APP_USER_AGENT)
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_log();

    let mut faktory = faktory::ConsumerBuilder::default();
    faktory.workers(2);

    faktory.register("new_submission", move |job| -> Result<(), WebhookError> {
        let _span = tracing::info_span!("new_submission", job_id = job.id()).entered();

        tracing::trace!("Got job");

        let data = job
            .args()
            .into_iter()
            .next()
            .ok_or(WebhookError::MissingData)?
            .to_owned()
            .to_owned();

        let value: fuzzysearch_common::types::WebHookData = serde_json::value::from_value(data)?;

        let mut conn = pool.get()?;

        for row in conn.query("SELECT endpoint FROM webhook", &[])? {
            let endpoint: &str = row.get(0);

            tracing::debug!(endpoint, "Sending webhook");

            client
                .post(endpoint)
                .json(&value)
                .send()?
                .error_for_status()?;
        }

        tracing::info!("Processed webhooks");

        Ok(())
    });

    let faktory = faktory.connect(None).unwrap_or_log();
    faktory.run_to_completion(&["fuzzysearch_webhook"]);
}
