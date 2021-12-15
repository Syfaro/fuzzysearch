use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// A wrapper around Faktory, providing an async interface to common operations.
#[derive(Clone)]
pub struct FaktoryClient {
    faktory: Arc<Mutex<faktory::Producer<TcpStream>>>,
}

impl FaktoryClient {
    /// Connect to a Faktory instance.
    pub async fn connect<H: Into<String>>(host: H) -> anyhow::Result<Self> {
        let host = host.into();

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
    pub async fn enqueue(&self, mut job: faktory::Job) -> anyhow::Result<()> {
        let faktory = self.faktory.clone();

        tracing::trace!("Attempting to enqueue job");
        job.custom = get_faktory_custom()
            .into_iter()
            .chain(job.custom.into_iter())
            .collect();

        tokio::task::spawn_blocking(move || {
            let mut faktory = faktory.lock().unwrap();
            faktory
                .enqueue(job)
                .map_err(|err| anyhow::format_err!("Unable to enqueue job: {:?}", err))
        })
        .await??;

        tracing::debug!("Enqueued job");

        Ok(())
    }

    /// Create a new job for webhook data and enqueue it.
    pub async fn queue_webhook(&self, data: WebHookData) -> anyhow::Result<()> {
        let value = serde_json::value::to_value(data)?;
        let mut job =
            faktory::Job::new("new_submission", vec![value]).on_queue("fuzzysearch_webhook");
        job.retry = Some(3);
        job.reserve_for = Some(30);
        self.enqueue(job).await
    }
}

fn get_faktory_custom() -> HashMap<String, serde_json::Value> {
    use opentelemetry::propagation::TextMapPropagator;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let context = tracing::Span::current().context();

    let mut extra: HashMap<String, String> = Default::default();
    let propagator = opentelemetry::sdk::propagation::TraceContextPropagator::new();
    propagator.inject_context(&context, &mut extra);

    extra
        .into_iter()
        .filter_map(|(key, value)| match serde_json::to_value(value) {
            Ok(val) => Some((key, val)),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebHookData {
    pub site: crate::types::Site,
    #[serde(with = "string")]
    pub site_id: i64,
    pub artist: String,
    pub file_url: String,
    #[serde(with = "b64_vec")]
    pub file_sha256: Option<Vec<u8>>,
    #[serde(with = "b64_u8")]
    pub hash: Option<[u8; 8]>,
}

mod b64_vec {
    use serde::Deserialize;

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match bytes {
            Some(bytes) => serializer.serialize_str(&base64::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = <Option<String>>::deserialize(deserializer)?
            .map(base64::decode)
            .transpose()
            .map_err(serde::de::Error::custom)?;

        Ok(val)
    }
}

mod b64_u8 {
    use std::convert::TryInto;

    use serde::Deserialize;

    pub fn serialize<S, const N: usize>(
        bytes: &Option<[u8; N]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match bytes {
            Some(bytes) => serializer.serialize_str(&base64::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<Option<[u8; N]>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = <Option<String>>::deserialize(deserializer)?
            .map(base64::decode)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .map(|bytes| bytes.try_into())
            .transpose()
            .map_err(|_err| "value did not have correct number of bytes")
            .map_err(serde::de::Error::custom)?;

        Ok(val)
    }
}

pub mod string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Display,
        S: Serializer,
    {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}
