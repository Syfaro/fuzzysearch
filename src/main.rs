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
) -> Result<(), postgres::Error> {
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

async fn insert_null_submission(client: &Client, id: i32) -> Result<u64, postgres::Error> {
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

    let addr: std::net::SocketAddr = std::env::var("HTTP_HOST").unwrap().parse().unwrap();

    let service = make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(request)) });

    let server = hyper::Server::bind(&addr).serve(service);

    println!("Listening on http://{}", addr);

    server.await.unwrap();
}

#[tokio::main]
async fn main() {
    let (cookie_a, cookie_b) = (
        std::env::var("FA_A").expect("missing fa cookie a"),
        std::env::var("FA_B").expect("missing fa cookie b"),
    );

    let user_agent = std::env::var("USER_AGENT").expect("missing user agent");

    let fa = furaffinity_rs::FurAffinity::new(cookie_a, cookie_b, user_agent);

    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");

    let (client, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            panic!(e);
        }
    });

    tokio::spawn(async move { web().await });

    println!("Started");

    'main: loop {
        println!("Fetching latest ID");
        let latest_id = fa.latest_id().await.expect("unable to get latest id");

        for id in ids_to_check(&client, latest_id).await {
            'attempt: for attempt in 0..3 {
                if !has_submission(&client, id).await {
                    println!("loading submission {}", id);

                    let timer = SUBMISSION_DURATION.start_timer();

                    let sub = match fa.get_submission(id).await {
                        Ok(sub) => sub,
                        Err(e) => {
                            println!("got error: {:?}, retry {}", e.message, e.retry);
                            timer.stop_and_discard();
                            if e.retry {
                                tokio::time::delay_for(std::time::Duration::from_secs(attempt + 1))
                                    .await;
                                continue 'attempt;
                            } else {
                                println!("unrecoverable, exiting");
                                break 'main;
                            }
                        }
                    };

                    let sub = match sub {
                        Some(sub) => sub,
                        None => {
                            println!("did not exist");
                            timer.stop_and_discard();
                            insert_null_submission(&client, id).await.unwrap();
                            break 'attempt;
                        }
                    };

                    let sub = match fa.calc_image_hash(sub.clone()).await {
                        Ok(sub) => sub,
                        Err(e) => {
                            println!("unable to hash image: {:?}", e);
                            sub
                        }
                    };

                    timer.stop_and_record();

                    insert_submission(&client, &sub).await.unwrap();

                    break 'attempt;
                }

                println!("ran out of attempts");
            }
        }

        println!("completed fetch, waiting a minute before loading more");

        tokio::time::delay_for(std::time::Duration::from_secs(60)).await;
    }
}
