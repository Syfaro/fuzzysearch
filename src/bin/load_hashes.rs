use bb8::Pool;
use bb8_postgres::PostgresConnectionManager;
use futures::future::FutureExt;

struct NeededPost {
    id: i32,
    full_url: String,
}

async fn hash_url(
    client: std::sync::Arc<reqwest::Client>,
    url: String,
) -> Result<(img_hash::ImageHash<[u8; 8]>, i64), image::ImageError> {
    let data = client
        .get(&url)
        .send()
        .await
        .expect("unable to get url")
        .bytes()
        .await
        .expect("unable to get bytes");

    let hasher = furaffinity_rs::get_hasher();
    let image = match image::load_from_memory(&data) {
        Ok(image) => image,
        Err(e) => {
            println!("{:?}", &data[0..50]);
            return Err(e);
        }
    };

    let hash = hasher.hash_image(&image);
    let mut bytes: [u8; 8] = [0; 8];
    bytes.copy_from_slice(hash.as_bytes());

    let num = i64::from_be_bytes(bytes);

    println!("{} - {}", url, num);

    Ok((hash, num))
}

async fn load_next_posts(
    db: Pool<PostgresConnectionManager<tokio_postgres::NoTls>>,
) -> Vec<NeededPost> {
    db.get()
        .await
        .unwrap()
        .query(
            "SELECT
                    id,
                    data->'file'->>'url' file_url
                FROM
                    e621
                WHERE
                    hash IS NULL AND
                    hash_error IS NULL AND
                    data->'file'->>'ext' IN ('jpg', 'png') AND
                    data->'file'->>'url' <> '/images/deleted-preview.png'
                ORDER BY id DESC
                LIMIT 384",
            &[],
        )
        .await
        .expect("Unable to get posts")
        .into_iter()
        .map(|row| NeededPost {
            id: row.get("id"),
            full_url: row.get("file_url"),
        })
        .collect()
}

#[tokio::main]
async fn main() {
    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");

    use std::str::FromStr;
    let manager = PostgresConnectionManager::new(
        tokio_postgres::Config::from_str(&dsn).expect("unable to parse postgres dsn"),
        tokio_postgres::NoTls,
    );

    let pool = Pool::builder()
        .build(manager)
        .await
        .expect("unable to build pool");

    let client = reqwest::Client::builder()
        .user_agent("Syfaro test client syfaro@huefox.com")
        .build()
        .expect("Unable to build http client");
    let client = std::sync::Arc::new(client);

    loop {
        println!("running loop");

        let needed_posts = load_next_posts(pool.clone()).await;

        if needed_posts.is_empty() {
            println!("no posts, waiting a minute");
            tokio::time::delay_for(std::time::Duration::from_secs(60)).await;
            continue;
        }

        for chunk in needed_posts.chunks(8) {
            let futs = chunk.iter().map(|post| {
                let db = pool.clone();
                let client = client.clone();
                let id = post.id;

                hash_url(client, post.full_url.clone()).then(move |res| async move {
                    match res {
                        Ok((_hash, num)) => {
                            let mut conn = db.get().await.unwrap();

                            let tx = conn
                                .transaction()
                                .await
                                .expect("Unable to create transaction");

                            tx.execute("UPDATE e621 SET hash = $2 WHERE id = $1", &[&id, &num])
                                .await
                                .expect("Unable to update hash in database");

                            tx.execute(
                                "INSERT INTO hashes (e621_id, hash) VALUES ($1, $2)",
                                &[&id, &num],
                            )
                            .await
                            .expect("Unable to insert hash to hashes table");

                            tx.commit().await.expect("Unable to commit tx");

                            drop(conn);
                        }
                        Err(e) => {
                            let desc = e.to_string();
                            println!("[{}] hashing error - {}", id, desc);
                            db.get()
                                .await
                                .unwrap()
                                .execute(
                                    "UPDATE e621 SET hash_error = $2 WHERE id = $1",
                                    &[&id, &desc],
                                )
                                .await
                                .expect("Unable to update hash error in database");
                        }
                    };
                })
            });

            println!("joining futs");

            futures::future::join_all(futs).await;

            println!("futs completed");
        }
    }
}
