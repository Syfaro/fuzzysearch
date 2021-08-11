#[cfg(feature = "queue")]
pub mod faktory;
pub mod types;
#[cfg(feature = "video")]
pub mod video;

#[cfg(feature = "trace")]
pub mod trace;

/// Create an instance of img_hash with project defaults.
pub fn get_hasher() -> img_hash::Hasher<[u8; 8]> {
    use img_hash::{HashAlg::Gradient, HasherConfig};

    HasherConfig::with_bytes_type::<[u8; 8]>()
        .hash_alg(Gradient)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}

/// Initialize the logger. This should only be called by the running binary.
pub fn init_logger() {
    if matches!(std::env::var("LOG_FMT").as_deref(), Ok("json")) {
        tracing_subscriber::fmt::Subscriber::builder()
            .json()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_timer(tracing_subscriber::fmt::time::ChronoUtc::rfc3339())
            .init();
    } else {
        tracing_subscriber::fmt::init();
    }
}
