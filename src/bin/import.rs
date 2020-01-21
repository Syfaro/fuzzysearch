async fn load_page(
    client: &reqwest::Client,
    before_id: Option<i32>,
) -> (Vec<i32>, serde_json::Value) {
    println!("Loading page with before_id {:?}", before_id);

    let mut query: Vec<(&'static str, String)> =
        vec![("typed_tags", "true".into()), ("count", "320".into())];

    if let Some(before_id) = before_id {
        query.push(("before_id", before_id.to_string()));
        if before_id <= 14 {
            panic!("that's it.");
        }
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

    db.execute(
        "CREATE TABLE IF NOT EXISTS e621 (id INTEGER PRIMARY KEY, hash BIGINT, data JSONB, hash_error TEXT)",
        &[],
    )
    .await
    .expect("Unable to create table");

    db.execute(
        "CREATE OR REPLACE FUNCTION extract_post_data() RETURNS TRIGGER AS $$
        BEGIN
            NEW.id = NEW.data->'id';
            RETURN NEW;
        END $$
        LANGUAGE 'plpgsql'",
        &[],
    )
    .await
    .expect("Unable to create function");

    db.execute("DROP TRIGGER IF EXISTS call_extract_post_data ON e621", &[])
        .await
        .expect("Unable to drop trigger");
    db.execute("CREATE TRIGGER call_extract_post_data BEFORE INSERT ON e621 FOR EACH ROW EXECUTE PROCEDURE extract_post_data()", &[]).await.expect("Unable to create trigger");

    let mut min_id = db
        .query_one("SELECT MIN(id) FROM e621", &[])
        .await
        .map(|row| row.get("min"))
        .expect("Unable to get min post");

    let client = reqwest::Client::builder()
        .user_agent("Syfaro test client syfaro@huefox.com")
        .build()
        .expect("Unable to build http client");

    let mut now;

    loop {
        now = std::time::Instant::now();

        let (ids, post_data) = load_page(&client, min_id).await;
        min_id = ids.into_iter().min();

        db.execute(
            "INSERT INTO e621 (data) SELECT json_array_elements($1::json)",
            &[&post_data],
        )
        .await
        .expect("Unable to insert");

        let elapsed = now.elapsed().as_millis() as u64;
        if elapsed < 1000 {
            let delay = 1000 - elapsed;
            println!("delaying {}ms before loading next page", delay);
            tokio::time::delay_for(std::time::Duration::from_millis(delay)).await;
        }
    }
}
