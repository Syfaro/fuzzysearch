use std::{
    convert::TryInto,
    io::{BufReader, SeekFrom},
};

use actix_web::{post, web::Data, App, HttpRequest, HttpResponse, HttpServer, Responder};
use tempfile::tempfile;
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::Semaphore,
};
use tokio_stream::StreamExt;

lazy_static::lazy_static! {
    static ref IMAGE_LOADING_DURATION: prometheus::Histogram =
        prometheus::register_histogram!("fuzzysearch_image_image_loading_seconds", "Duration to download and save image").unwrap();
    static ref IMAGE_DECODING_DURATION: prometheus::Histogram =
        prometheus::register_histogram!("fuzzysearch_image_image_decoding_seconds", "Duration to decode image data").unwrap();
    static ref IMAGE_HASHING_DURATION: prometheus::Histogram =
        prometheus::register_histogram!("fuzzysearch_image_image_hashing_seconds", "Duration to hash image").unwrap();
}

enum ImageResponse {
    Hash(i64),
    Error(anyhow::Error),
}

impl Responder for ImageResponse {
    fn respond_to(self, _req: &HttpRequest) -> HttpResponse {
        match self {
            ImageResponse::Hash(hash) => HttpResponse::Ok()
                .content_type("text/plain")
                .body(hash.to_string()),
            ImageResponse::Error(error) => HttpResponse::BadRequest()
                .content_type("text/plain")
                .body(error.to_string()),
        }
    }
}

#[tracing::instrument(err, skip(field, semaphore))]
async fn process_image(
    mut field: actix_multipart::Field,
    semaphore: Data<Semaphore>,
) -> anyhow::Result<i64> {
    tracing::debug!("creating temp file");

    let loading_duration = IMAGE_LOADING_DURATION.start_timer();
    let mut file =
        tokio::task::spawn_blocking(move || -> anyhow::Result<tokio::fs::File, anyhow::Error> {
            let file = tempfile()?;
            Ok(tokio::fs::File::from_std(file))
        })
        .await??;

    tracing::debug!("writing contents to temp file");
    let mut size = 0;
    while let Ok(Some(chunk)) = field.try_next().await {
        file.write_all(&chunk).await?;
        size += chunk.len();
    }
    tracing::debug!("file was {} bytes", size);

    tracing::debug!("returning file to beginning");
    file.seek(SeekFrom::Start(0)).await?;
    let file = file.into_std().await;
    loading_duration.stop_and_record();

    tracing::debug!("getting semaphore permit");
    let _permit = semaphore.acquire().await?;

    tracing::debug!("decoding and hashing image");
    let hash = tokio::task::spawn_blocking(move || -> anyhow::Result<i64, anyhow::Error> {
        let decoding_duration = IMAGE_DECODING_DURATION.start_timer();
        let reader = BufReader::new(file);
        let reader = image::io::Reader::new(reader).with_guessed_format()?;
        let im = reader.decode()?;
        decoding_duration.stop_and_record();

        let hashing_duration = IMAGE_HASHING_DURATION.start_timer();
        let image_hash = fuzzysearch_common::get_hasher().hash_image(&im);
        let hash: [u8; 8] = image_hash.as_bytes().try_into()?;
        let hash = i64::from_be_bytes(hash);
        hashing_duration.stop_and_record();

        Ok(hash)
    })
    .await??;

    tracing::debug!("calculated image hash: {}", hash);
    Ok(hash)
}

#[post("/image")]
async fn post_image(
    mut form: actix_multipart::Multipart,
    semaphore: Data<Semaphore>,
) -> impl Responder {
    while let Ok(Some(field)) = form.try_next().await {
        tracing::debug!("got multipart field: {:?}", field);

        if !matches!(field.content_disposition().get_name(), Some("image")) {
            continue;
        }

        match process_image(field, semaphore).await {
            Ok(hash) => return ImageResponse::Hash(hash),
            Err(err) => return ImageResponse::Error(err),
        }
    }

    ImageResponse::Error(anyhow::anyhow!("missing image field"))
}

#[actix_web::main]
async fn main() {
    fuzzysearch_common::trace::configure_tracing("fuzzysearch-image");
    fuzzysearch_common::trace::serve_metrics().await;

    let semaphore = Data::new(Semaphore::new(4));

    HttpServer::new(move || {
        App::new()
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(semaphore.clone())
            .service(post_image)
    })
    .workers(2)
    .bind("0.0.0.0:8090")
    .unwrap()
    .run()
    .await
    .unwrap();
}
