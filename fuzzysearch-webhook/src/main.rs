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
    #[error("faktory error")]
    Faktory,
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

    let producer = std::sync::Mutex::new(faktory::Producer::connect(None).unwrap());

    faktory.register("new_submission", move |job| -> Result<(), WebhookError> {
        let _span = tracing::info_span!("new_submission", job_id = job.id()).entered();

        let data = job
            .args()
            .iter()
            .next()
            .ok_or(WebhookError::MissingData)?
            .to_owned();

        let mut conn = pool.get()?;

        for row in conn.query("SELECT endpoint FROM webhook", &[])? {
            let endpoint: &str = row.get(0);

            tracing::debug!(endpoint, "Queueing webhook");

            let job = faktory::Job::new(
                "send_webhook",
                vec![data.clone(), serde_json::to_value(endpoint)?],
            )
            .on_queue("fuzzysearch_webhook");

            let mut producer = producer.lock().unwrap();
            producer.enqueue(job).map_err(|_| WebhookError::Faktory)?;
        }

        tracing::info!("Queued webhooks");

        Ok(())
    });

    faktory.register("send_webhook", move |job| -> Result<(), WebhookError> {
        let _span = tracing::info_span!("send_webhook", job_id = job.id()).entered();

        let mut args = job.args().iter();

        let data = args.next().ok_or(WebhookError::MissingData)?.to_owned();
        let value: fuzzysearch_common::types::WebHookData = serde_json::value::from_value(data)?;

        let endpoint = args
            .next()
            .ok_or(WebhookError::MissingData)?
            .as_str()
            .ok_or(WebhookError::MissingData)?;

        tracing::trace!(endpoint, site = %value.site, site_id = value.site_id, "Sending webhook");

        client
            .post(endpoint)
            .json(&value)
            .send()?
            .error_for_status()?;

        Ok(())
    });

    let faktory = faktory.connect(None).unwrap_or_log();
    faktory.run_to_completion(&["fuzzysearch_webhook"]);
}
