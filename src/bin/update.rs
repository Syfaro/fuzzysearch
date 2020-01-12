async fn load_page(
    client: &reqwest::Client,
    before_id: Option<i32>,
) -> (Vec<i32>, serde_json::Value) {
    println!("Loading page with before_id {:?}", before_id);

    let mut query: Vec<(&'static str, String)> =
        vec![("typed_tags", "true".into()), ("count", "320".into())];

    if let Some(before_id) = before_id {
        query.push(("before_id", before_id.to_string()));
    }

    let body = client
        .get("https://e621.net/post/index.json")
        .query(&query)
        .send()
        .await
        .expect("unable to make request")
        .text()
        .await
        .expect("unable to convert to text");

    let json = serde_json::from_str(&body).expect("Unable to parse data");

    let posts = match json {
        serde_json::Value::Array(ref arr) => arr,
        _ => panic!("invalid response"),
    };

    let ids = posts
        .iter()
        .map(|post| {
            let post = match post {
                serde_json::Value::Object(post) => post,
                _ => panic!("invalid post data"),
            };

            match post.get("id").expect("missing post id") {
                serde_json::Value::Number(num) => {
                    num.as_i64().expect("invalid post id type") as i32
                }
                _ => panic!("invalid post id"),
            }
        })
        .collect();

    (ids, json)
}

#[tokio::main]
async fn main() {
    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");

    let (db, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .expect("Unable to connect");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    let max_id: i32 = db
        .query_one("SELECT max(id) FROM post", &[])
        .await
        .map(|row| row.get("max"))
        .expect("Unable to get max post");

    let client = reqwest::Client::builder()
        .user_agent("Syfaro test client syfaro@huefox.com")
        .build()
        .expect("Unable to build http client");

    let mut now;
    let mut min_id: Option<i32> = None;

    loop {
        now = std::time::Instant::now();

        let (ids, post_data) = load_page(&client, min_id).await;
        min_id = ids.into_iter().min();

        db.execute(
            "INSERT INTO post (data) SELECT json_array_elements($1::json) ON CONFLICT DO NOTHING",
            &[&post_data],
        )
        .await
        .expect("Unable to insert");

        if let Some(min_id) = min_id {
            if min_id >= max_id {
                println!("finished run, {}, {}", min_id, max_id);
                break
            }
        }

        let elapsed = now.elapsed().as_millis() as u64;
        if elapsed < 1000 {
            let delay = 1000 - elapsed;
            println!("delaying {}ms before loading next page", delay);
            tokio::time::delay_for(std::time::Duration::from_millis(delay)).await;
        }
    }
}
