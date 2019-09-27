type Client = r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager>;

fn lookup_tag(client: &Client, tag: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM tag WHERE name = $1", &[&tag])
        .unwrap()
        .into_iter()
        .next()
    {
        return row.get("id");
    }

    client
        .query("INSERT INTO tag (name) VALUES ($1) RETURNING id", &[&tag])
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .get("id")
}

fn lookup_artist(client: &Client, artist: &str) -> i32 {
    if let Some(row) = client
        .query("SELECT id FROM artist WHERE name = $1", &[&artist])
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
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .get("id")
}

fn has_submission(client: &Client, id: i32) -> bool {
    client
        .query("SELECT id FROM submission WHERE id = $1", &[&id])
        .expect("unable to run query")
        .into_iter()
        .next()
        .is_some()
}

fn ids_to_check(client: &Client, max: i32) -> Vec<i32> {
    let min = max - 100;

    let rows = client.query("SELECT sid FROM generate_series(LEAST($1::int, (SELECT MIN(id) FROM SUBMISSION)), $2::int) sid WHERE sid NOT IN (SELECT id FROM submission where id = sid)", &[&min, &max]).unwrap();

    rows.iter().map(|row| row.get("sid")).collect()
}

fn insert_submission(
    client: &Client,
    sub: &furaffinity_rs::Submission,
) -> Result<(), postgres::Error> {
    let artist_id = lookup_artist(&client, &sub.artist);
    let tag_ids: Vec<i32> = sub
        .tags
        .iter()
        .map(|tag| lookup_tag(&client, &tag))
        .collect();

    let hash = sub.hash.clone();

    let url = match &sub.content {
        furaffinity_rs::Content::Image(url) => url,
        furaffinity_rs::Content::Flash(url) => url,
    };

    client.execute("INSERT INTO submission (id, artist_id, url, filename, hash, rating, posted_at, description, hash1, hash2, hash3, hash4, hash5, hash6, hash7, hash8) VALUES ($1, $2, $3, $4, decode($5, 'base64'), $6, $7, $8, get_byte(decode($5, 'base64'), 0), get_byte(decode($5, 'base64'), 1), get_byte(decode($5, 'base64'), 2), get_byte(decode($5, 'base64'), 3), get_byte(decode($5, 'base64'), 4), get_byte(decode($5, 'base64'), 5), get_byte(decode($5, 'base64'), 6), get_byte(decode($5, 'base64'), 7))", &[
        &sub.id, &artist_id, &url, &sub.filename, &hash, &sub.rating.serialize(), &sub.posted_at, &sub.description,
    ])?;

    let stmt = client.prepare(
        "INSERT INTO tag_to_post (tag_id, post_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )?;
    for tag_id in tag_ids {
        stmt.execute(&[&tag_id, &sub.id])?;
    }

    Ok(())
}

fn insert_null_submission(client: &Client, id: i32) -> Result<u64, postgres::Error> {
    client.execute("INSERT INTO SUBMISSION (id) VALUES ($1)", &[&id])
}

fn main() {
    let (cookie_a, cookie_b) = (
        std::env::var("FA_A").expect("missing fa cookie a"),
        std::env::var("FA_B").expect("missing fa cookie b"),
    );

    let user_agent = std::env::var("USER_AGENT").expect("missing user agent");

    let fa = furaffinity_rs::FurAffinity::new(cookie_a, cookie_b, user_agent);

    let dsn = std::env::var("POSTGRES_DSN").expect("missing postgres dsn");

    let manager =
        r2d2_postgres::PostgresConnectionManager::new(dsn, r2d2_postgres::TlsMode::None).unwrap();

    let pool = r2d2::Pool::new(manager).unwrap();

    'main: loop {
        let client = pool.get().unwrap();

        let latest_id = fa.latest_id().expect("unable to get latest id");

        for id in ids_to_check(&client, latest_id) {
            'attempt: for attempt in 0..3 {
                if !has_submission(&client, id) {
                    println!("loading submission {}", id);

                    let sub = match fa.get_submission(id) {
                        Ok(sub) => sub,
                        Err(e) => {
                            println!("got error: {:?}, retry {}", e.message, e.retry);
                            if e.retry {
                                std::thread::sleep(std::time::Duration::from_secs(attempt + 1));
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
                            insert_null_submission(&client, id).unwrap();
                            break 'attempt;
                        }
                    };

                    let sub = match fa.calc_image_hash(sub.clone()) {
                        Ok(sub) => sub,
                        Err(e) => {
                            println!("unable to hash image: {:?}", e);
                            sub
                        },
                    };

                    insert_submission(&client, &sub).unwrap();

                    break 'attempt;
                }

                println!("ran out of attempts");
            }
        }

        println!("completed fetch, waiting a minute before loading more");

        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
