async fn load_page(client: &reqwest::Client, after_id: i32) -> (Vec<i32>, Vec<serde_json::Value>) {
    println!("Loading page with after_id {:?}", after_id);

    let mut query: Vec<(&'static str, String)> = vec![("limit", "320".into())];
    query.push(("page", format!("a{}", after_id)));

    let body = client
        .get("https://e621.net/posts.json")
        .query(&query)
        .send()
        .await
        .expect("unable to make request")
        .text()
        .await
        .expect("unable to convert to text");

    let json = serde_json::from_str(&body).expect("Unable to parse data");

    let page = match json {
        serde_json::Value::Object(ref obj) => obj,
        _ => panic!("top level value was not object"),
    };

    let posts = page
        .get("posts")
        .expect("unable to get posts object")
        .as_array()
        .expect("posts was not array");

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

    (ids, posts.to_vec())
}

async fn get_latest_id(client: &reqwest::Client) -> i32 {
    println!("Looking up current highest ID");

    let query = vec![("limit", "1")];

    let body = client
        .get("https://e621.net/posts.json")
        .query(&query)
        .send()
        .await
        .expect("unable to make request")
        .text()
        .await
        .expect("unable to convert to text");

    let json = serde_json::from_str(&body).expect("Unable to parse data");

    let page = match json {
        serde_json::Value::Object(ref obj) => obj,
        _ => panic!("top level value was not object"),
    };

    let posts = page
        .get("posts")
        .expect("unable to get posts object")
        .as_array()
        .expect("posts was not array");

    let ids: Vec<i32> = posts
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

    ids.into_iter().max().expect("no ids found")
}

#[tokio::main]
async fn main() {
    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");

    let (mut db, connection) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
        .await
        .expect("Unable to connect");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    let max_id: i32 = db
        .query_one("SELECT max(id) FROM e621", &[])
        .await
        .map(|row| row.get("max"))
        .expect("Unable to get max post");

    let client = reqwest::Client::builder()
        .user_agent("Syfaro test client syfaro@huefox.com")
        .build()
        .expect("Unable to build http client");

    println!("max is id: {}", max_id);

    let mut now;

    // Start with the minimum ID we're requesting being our previous highest
    // ID found.
    let mut min_id = max_id;

    // Find highest ID to look for. Once we get this value back, we've gotten
    // as many new posts as we were looking for.
    let latest_id = get_latest_id(&client).await;

    loop {
        now = std::time::Instant::now();

        // Load any posts with an ID higher than our previous run.
        let (ids, post_data) = load_page(&client, min_id).await;

        // Calculate a new minimum value to find posts after by looking at the
        // maximum value returned in this run.
        min_id = *ids.iter().max().expect("no ids found");

        let tx = db.transaction().await.expect("unable to start transaction");

        for post in post_data {
            tx.execute(
                "INSERT INTO e621 (data) VALUES ($1::json) ON CONFLICT DO NOTHING",
                &[&post],
            )
            .await
            .expect("Unable to insert");
        }

        tx.commit().await.expect("unable to commit transaction");

        // If it contains the latest ID, we're done.
        if ids.contains(&latest_id) {
            println!("finished run, latest_id {}, max_id {}", latest_id, max_id);
            break;
        }

        let elapsed = now.elapsed().as_millis() as u64;
        if elapsed < 1000 {
            let delay = 1000 - elapsed;
            println!("delaying {}ms before loading next page", delay);
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
    }
}
