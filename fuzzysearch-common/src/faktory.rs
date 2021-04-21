use std::net::TcpStream;
use std::sync::{Arc, Mutex};

/// A wrapper around Faktory, providing an async interface to common operations.
pub struct FaktoryClient {
    faktory: Arc<Mutex<faktory::Producer<TcpStream>>>,
}

impl FaktoryClient {
    /// Connect to a Faktory instance.
    pub async fn connect(host: String) -> anyhow::Result<Self> {
        let producer = tokio::task::spawn_blocking(move || {
            faktory::Producer::connect(Some(&host))
                .map_err(|err| anyhow::format_err!("Unable to connect to Faktory: {:?}", err))
        })
        .await??;

        let faktory = Arc::new(Mutex::new(producer));

        Ok(FaktoryClient { faktory })
    }

    /// Enqueue a new job.
    #[tracing::instrument(err, skip(self))]
    async fn enqueue(&self, job: faktory::Job) -> anyhow::Result<()> {
        let faktory = self.faktory.clone();

        tracing::trace!("Attempting to enqueue webhook data");

        tokio::task::spawn_blocking(move || {
            let mut faktory = faktory.lock().unwrap();
            faktory
                .enqueue(job)
                .map_err(|err| anyhow::format_err!("Unable to enqueue job: {:?}", err))
        })
        .await??;

        tracing::debug!("Enqueued webhook data");

        Ok(())
    }

    /// Create a new job for webhook data and enqueue it.
    pub async fn queue_webhook(&self, data: crate::types::WebHookData) -> anyhow::Result<()> {
        let value = serde_json::value::to_value(data)?;
        let mut job =
            faktory::Job::new("new_submission", vec![value]).on_queue("fuzzysearch_webhook");
        job.retry = Some(3);
        job.reserve_for = Some(30);
        self.enqueue(job).await
    }
}
