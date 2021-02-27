use lazy_static::lazy_static;
use tokio_postgres::Client;

lazy_static! {
    static ref SUBMISSION_DURATION: prometheus::Histogram = prometheus::register_histogram!(
        "fuzzysearch_watcher_fa_processing_seconds",
        "Duration to process a submission"
    )
    .unwrap();
}

async fn lookup_tag(client: &Client, tag: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM tag WHERE name = $1", &[&tag])
        .await
        .unwrap()
        .into_iter()
        .next()
    {
        return row.get("id");
    }

    client
        .query("INSERT INTO tag (name) VALUES ($1) RETURNING id", &[&tag])
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .get("id")
}

async fn lookup_artist(client: &Client, artist: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM artist WHERE name = $1", &[&artist])
        .await
        .unwrap()
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
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .get("id")
}

async fn has_submission(client: &Client, id: i32) -> bool {
    client
        .query("SELECT id FROM submission WHERE id = $1", &[&id])
        .await
        .expect("unable to run query")
        .into_iter()
        .next()
        .is_some()
}

async fn ids_to_check(client: &Client, max: i32) -> Vec<i32> {
    let rows = client.query("SELECT sid FROM generate_series((SELECT max(id) FROM submission), $1::int) sid WHERE sid NOT IN (SELECT id FROM submission where id = sid)", &[&max]).await.unwrap();

    rows.iter().map(|row| row.get("sid")).collect()
}

async fn insert_submission(
    client: &Client,
    sub: &furaffinity_rs::Submission,
) -> Result<(), tokio_postgres::Error> {
    let artist_id = lookup_artist(&client, &sub.artist).await;
    let mut tag_ids = Vec::with_capacity(sub.tags.len());
    for tag in &sub.tags {
        tag_ids.push(lookup_tag(&client, &tag).await);
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

async fn request(
    req: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/health") => Ok(hyper::Response::new(hyper::Body::from("OK"))),

        (&hyper::Method::GET, "/metrics") => {
            use prometheus::Encoder;

            let encoder = prometheus::TextEncoder::new();

            let metric_families = prometheus::gather();
            let mut buffer = vec![];
            encoder.encode(&metric_families, &mut buffer).unwrap();

            Ok(hyper::Response::new(hyper::Body::from(buffer)))
        }

        _ => {
            let mut not_found = hyper::Response::default();
            *not_found.status_mut() = hyper::StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

async fn web() {
    use hyper::service::{make_service_fn, service_fn};

    let addr: std::net::SocketAddr = std::env::var("HTTP_HOST")
        .expect("Missing HTTP_HOST")
        .parse()
        .unwrap();

    let service = make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(request)) });

    let server = hyper::Server::bind(&addr).serve(service);

    println!("Listening on http://{}", addr);

    server.await.unwrap();
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

    fn handle(
        &mut self,
        attempt: usize,
        err: furaffinity_rs::Error,
    ) -> futures_retry::RetryPolicy<Self::OutError> {
        println!(
            "Attempt {}/{} has failed: {:?}",
            attempt, self.max_attempts, err
        );

        if attempt >= self.max_attempts {
            println!("All attempts have been used");
            return futures_retry::RetryPolicy::ForwardError(err);
        }

        if !err.retry {
            println!("Error did not ask for retry");
            return futures_retry::RetryPolicy::ForwardError(err);
        }

        return futures_retry::RetryPolicy::WaitRetry(std::time::Duration::from_secs(
            1 + attempt as u64,
        ));
    }
}

#[tokio::main]
async fn main() {
    let (cookie_a, cookie_b) = (
        std::env::var("FA_A").expect("Missing FA_A"),
        std::env::var("FA_B").expect("Missing FA_B"),
    );

    let user_agent = std::env::var("USER_AGENT").expect("Missing USER_AGENT");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let fa = std::sync::Arc::new(furaffinity_rs::FurAffinity::new(
        cookie_a,
        cookie_b,
        user_agent,
        Some(client),
    ));

    let dsn = std::env::var("POSTGRES_DSN").expect("Missing POSTGRES_DSN");

    let (client, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            panic!("PostgreSQL connection error: {:?}", e);
        }
    });

    tokio::spawn(async move { web().await });

    println!("Started");

    loop {
        print!("Fetching latest ID... ");
        let latest_id = fa.latest_id().await.expect("unable to get latest id");
        println!("{}", latest_id);

        for id in ids_to_check(&client, latest_id).await {
            if has_submission(&client, id).await {
                continue;
            }

            println!("Loading submission {}", id);

            let _timer = SUBMISSION_DURATION.start_timer();

            let sub =
                futures_retry::FutureRetry::new(|| fa.get_submission(id), RetryHandler::new(3))
                    .await
                    .map(|(sub, _attempts)| sub)
                    .map_err(|(err, _attempts)| err);

            let sub = match sub {
                Ok(sub) => sub,
                Err(err) => {
                    println!("Failed to load submission {}: {:?}", id, err);
                    _timer.stop_and_discard();
                    insert_null_submission(&client, id).await.unwrap();
                    continue;
                }
            };

            let sub = match sub {
                Some(sub) => sub,
                None => {
                    println!("Submission {} did not exist", id);
                    _timer.stop_and_discard();
                    insert_null_submission(&client, id).await.unwrap();
                    continue;
                }
            };

            let image = futures_retry::FutureRetry::new(
                || fa.calc_image_hash(sub.clone()),
                RetryHandler::new(3),
            )
            .await
            .map(|(sub, _attempt)| sub)
            .map_err(|(err, _attempt)| err);

            let sub = match image {
                Ok(sub) => sub,
                Err(err) => {
                    println!("Unable to hash submission {} image: {:?}", id, err);
                    sub
                }
            };

            _timer.stop_and_record();

            insert_submission(&client, &sub).await.unwrap();
        }

        println!("Completed fetch, waiting a minute before loading more");
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
