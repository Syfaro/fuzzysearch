#[derive(Debug)]
struct Row {
    id: i32,
    artists: Option<Vec<String>>,
    sources: Option<Vec<String>>,
    distance: Option<u64>,
}

async fn get_hash_distance_from_url(
    client: &reqwest::Client,
    url: &str,
    other: &img_hash::ImageHash,
) -> Result<u32, Box<dyn std::error::Error>> {
    let data = client.get(url).send().await?.bytes().await?;

    let hasher = furaffinity_rs::get_hasher();
    let image = image::load_from_memory(&data)?;

    let hash = hasher.hash_image(&image);
    Ok(hash.dist(&other))
}

#[tokio::main]
async fn main() {
    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");
    let file = std::env::args().nth(1).expect("missing image");

    let (db, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .expect("Unable to connect");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    let client = reqwest::Client::builder()
        .user_agent("Syfaro test client syfaro@huefox.com")
        .build()
        .expect("Unable to build http client");

    let image = image::open(&file).expect("unable to open image");

    let hasher = furaffinity_rs::get_hasher();
    let hash = hasher.hash_image(&image);

    let mut bytes: [u8; 8] = [0; 8];
    bytes.copy_from_slice(hash.as_bytes());

    let num = i64::from_be_bytes(bytes);

    let rows = db
        .query(
            "SELECT
                post.id id,
                post.hash hash,
                artists_agg.artists artists,
                sources_agg.sources sources
            FROM
                post,
                LATERAL (
                    SELECT array_agg(v) artists FROM jsonb_array_elements_text(data->'artist') v
                ) artists_agg,
                LATERAL (
                    SELECT array_agg(v) sources FROM jsonb_array_elements_text(data->'sources') v
                ) sources_agg
            WHERE hash <@ ($1, 10)",
            &[&num],
        )
        .await
        .expect("unable to query")
        .into_iter()
        .map(|row| {
            let distance = row
                .get::<&str, Option<i64>>("hash")
                .map(|hash| hamming::distance_fast(&hash.to_be_bytes(), &bytes).unwrap());

            Row {
                id: row.get("id"),
                sources: row.get("sources"),
                artists: row.get("artists"),
                distance,
            }
        });

    for row in rows {
        println!(
            "Possible match: [distance of {}] https://e621.net/post/show/{} by {}",
            row.distance.unwrap_or_else(u64::max_value),
            row.id,
            row.artists
                .map(|artists| artists.join(", "))
                .unwrap_or_else(|| "unknown".to_string())
        );
        let sources = match row.sources {
            Some(source) => source,
            _ => {
                println!("no sources");
                continue;
            }
        };
        for source in sources {
            let distance = get_hash_distance_from_url(&client, &source, &hash).await;
            println!(
                "- {} (distance of {})",
                source,
                if let Ok(d) = distance {
                    d.to_string()
                } else {
                    "unknown".to_string()
                }
            );
        }
    }
}
