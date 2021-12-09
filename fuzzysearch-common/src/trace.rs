pub fn configure_tracing(service_name: &'static str) {
    use opentelemetry::KeyValue;
    use tracing_subscriber::layer::SubscriberExt;

    tracing_log::LogTracer::init().unwrap();

    let env = std::env::var("ENVIRONMENT");
    let env = if let Ok(env) = env.as_ref() {
        env.as_str()
    } else if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());

    let tracer = opentelemetry_jaeger::new_pipeline()
        .with_agent_endpoint(std::env::var("JAEGER_COLLECTOR").expect("Missing JAEGER_COLLECTOR"))
        .with_service_name(service_name)
        .with_tags(vec![
            KeyValue::new("environment", env.to_owned()),
            KeyValue::new("version", env!("CARGO_PKG_VERSION")),
        ])
        .install_batch(opentelemetry::runtime::Tokio)
        .unwrap();

    let trace = tracing_opentelemetry::layer().with_tracer(tracer);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    if matches!(std::env::var("LOG_FMT").as_deref(), Ok("json")) {
        let subscriber = tracing_subscriber::fmt::layer()
            .json()
            .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
            .with_target(true);
        let subscriber = tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(trace)
            .with(subscriber);
        tracing::subscriber::set_global_default(subscriber).unwrap();
    } else {
        let subscriber = tracing_subscriber::fmt::layer();
        let subscriber = tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(trace)
            .with(subscriber);
        tracing::subscriber::set_global_default(subscriber).unwrap();
    }

    tracing::debug!(service_name, "set application tracing service name");
}

async fn metrics(
    req: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, std::convert::Infallible> {
    use hyper::{Body, Response, StatusCode};

    match req.uri().path() {
        "/health" => Ok(Response::new(Body::from("OK"))),
        "/metrics" => {
            use prometheus::{Encoder, TextEncoder};

            let mut buffer = Vec::new();
            let encoder = TextEncoder::new();

            let metric_families = prometheus::gather();
            encoder.encode(&metric_families, &mut buffer).unwrap();

            Ok(Response::new(Body::from(buffer)))
        }
        _ => {
            let mut not_found = Response::new(Body::default());
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

pub async fn serve_metrics() {
    use hyper::{
        server::Server,
        service::{make_service_fn, service_fn},
    };
    use std::convert::Infallible;
    use std::net::SocketAddr;

    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(metrics)) });

    let addr: SocketAddr = std::env::var("METRICS_HOST")
        .expect("Missing METRICS_HOST")
        .parse()
        .expect("Invalid METRICS_HOST");

    let server = Server::bind(&addr).serve(make_svc);

    tokio::spawn(async move {
        server.await.expect("Metrics server error");
    });
}

pub trait InjectContext {
    fn inject_context(self) -> Self;
}

impl InjectContext for reqwest::RequestBuilder {
    fn inject_context(self: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let mut headers: reqwest::header::HeaderMap = Default::default();

        let cx = tracing::Span::current().context();
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&cx, &mut opentelemetry_http::HeaderInjector(&mut headers))
        });

        self.headers(headers)
    }
}
