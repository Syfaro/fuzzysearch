use tracing_unwrap::{OptionExt, ResultExt};

use r2d2_postgres::{postgres::NoTls, PostgresConnectionManager};

static APP_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " - ",
    env!("CARGO_PKG_AUTHORS")
);

fn main() {
    tracing_subscriber::fmt::init();

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

    faktory.register("new_submission", move |job| -> Result<(), reqwest::Error> {
        let _span = tracing::info_span!("new_submission", job_id = job.id()).entered();

        tracing::trace!("Got job");

        let value: fuzzysearch_common::types::WebHookData =
            serde_json::value::from_value(job.args().into_iter().next().unwrap_or_log().to_owned())
                .unwrap_or_log();

        let mut conn = pool.get().unwrap_or_log();

        for row in conn
            .query(
                "
                SELECT endpoint
                FROM webhook
                WHERE
                    filtered = false OR
                    exists(
                        SELECT 1
                        FROM webhook_filter
                        WHERE
                            webhook_id = webhook.id AND
                            site_id = $1 AND
                            artist_name = $2
                    )",
                &[&value.site, &value.artist],
            )
            .unwrap_or_log()
        {
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
