use crate::types::*;
use crate::utils::extract_rows;
use crate::Pool;

pub type DB<'a> =
    &'a bb8::PooledConnection<'a, bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>>;

pub async fn lookup_api_key(key: &str, db: DB<'_>) -> Option<ApiKey> {
    let rows = db
        .query(
            "SELECT
            api_key.id,
            api_key.name_limit,
            api_key.image_limit,
            api_key.name,
            account.email
        FROM
            api_key
        JOIN account
            ON account.id = api_key.user_id
        WHERE
            api_key.key = $1",
            &[&key],
        )
        .await
        .expect("Unable to query API keys");

    match rows.into_iter().next() {
        Some(row) => Some(ApiKey {
            id: row.get(0),
            name_limit: row.get(1),
            image_limit: row.get(2),
            name: row.get(3),
            owner_email: row.get(4),
        }),
        _ => None,
    }
}

pub async fn image_query(
    pool: Pool,
    hashes: Vec<i64>,
    distance: i64,
    hash: Option<Vec<u8>>,
) -> Result<Vec<File>, tokio_postgres::Error> {
    let mut results = image_query_sync(pool, hashes, distance, hash);
    let mut matches = Vec::new();

    while let Some(r) = results.recv().await {
        matches.extend(r?);
    }

    Ok(matches)
}

pub fn image_query_sync(
    pool: Pool,
    hashes: Vec<i64>,
    distance: i64,
    hash: Option<Vec<u8>>,
) -> tokio::sync::mpsc::Receiver<Result<Vec<File>, tokio_postgres::Error>> {
    let (mut tx, rx) = tokio::sync::mpsc::channel(1);

    tokio::spawn(async move {
        let db = pool.get().await.unwrap();

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            Vec::with_capacity(hashes.len() + 1);
        params.insert(0, &distance);

        let mut hash_where_clause = Vec::with_capacity(hashes.len());
        for (idx, hash) in hashes.iter().enumerate() {
            params.push(hash);
            hash_where_clause.push(format!(" hash <@ (${}, $1)", idx + 2));
        }
        let hash_where_clause = hash_where_clause.join(" OR ");

        let hash_query = format!(
            "SELECT
                hashes.id,
                hashes.hash,
                hashes.furaffinity_id,
                hashes.e621_id,
                hashes.twitter_id,
            CASE
                WHEN furaffinity_id IS NOT NULL THEN (f.url)
                WHEN e621_id IS NOT NULL THEN (e.data->>'file_url')
                WHEN twitter_id IS NOT NULL THEN (tm.url)
             END url,
             CASE
                WHEN furaffinity_id IS NOT NULL THEN (f.filename)
                WHEN e621_id IS NOT NULL THEN ((e.data->>'md5') || '.' || (e.data->>'file_ext'))
                WHEN twitter_id IS NOT NULL THEN (SELECT split_part(split_part(tm.url, '/', 5), ':', 1))
             END filename,
            CASE
                WHEN furaffinity_id IS NOT NULL THEN (ARRAY(SELECT f.name))
                WHEN e621_id IS NOT NULL THEN ARRAY(SELECT jsonb_array_elements_text(e.data->'artist'))
                WHEN twitter_id IS NOT NULL THEN ARRAY(SELECT tw.data->'user'->>'screen_name')
            END artists,
            CASE
                WHEN furaffinity_id IS NOT NULL THEN (f.file_id)
            END file_id,
            CASE
                WHEN e621_id IS NOT NULL THEN ARRAY(SELECT jsonb_array_elements_text(e.data->'sources'))
            END sources
        FROM
            hashes
        LEFT JOIN LATERAL (
            SELECT *
            FROM submission
            JOIN artist ON submission.artist_id = artist.id
            WHERE submission.id = hashes.furaffinity_id
        ) f ON hashes.furaffinity_id IS NOT NULL
        LEFT JOIN LATERAL (
            SELECT *
            FROM e621
            WHERE e621.id = hashes.e621_id
        ) e ON hashes.e621_id IS NOT NULL
        LEFT JOIN LATERAL (
            SELECT *
            FROM tweet
            WHERE tweet.id = hashes.twitter_id
        ) tw ON hashes.twitter_id IS NOT NULL
        LEFT JOIN LATERAL (
            SELECT *
            FROM tweet_media
            WHERE
                tweet_media.tweet_id = hashes.twitter_id AND
                tweet_media.hash = hashes.hash
        ) tm ON hashes.twitter_id IS NOT NULL
        WHERE {}", hash_where_clause);

        let query = db.query::<str>(&*hash_query, &params).await;
        let rows = query.map(|rows| extract_rows(rows, hash.as_deref()).into_iter().collect());
        tx.send(rows).await.unwrap();
    });

    rx
}
