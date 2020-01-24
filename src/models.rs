use crate::types::*;
use crate::utils::{extract_e621_rows, extract_fa_rows, extract_twitter_rows};
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
    use futures_util::FutureExt;

    let (mut tx, rx) = tokio::sync::mpsc::channel(3);

    tokio::spawn(async move {
        let db = pool.get().await.unwrap();

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            Vec::with_capacity(hashes.len() + 1);
        params.insert(0, &distance);

        let mut fa_where_clause = Vec::with_capacity(hashes.len());
        let mut hash_where_clause = Vec::with_capacity(hashes.len());

        for (idx, hash) in hashes.iter().enumerate() {
            params.push(hash);

            fa_where_clause.push(format!(" hash_int <@ (${}, $1)", idx + 2));
            hash_where_clause.push(format!(" hash <@ (${}, $1)", idx + 2));
        }
        let hash_where_clause = hash_where_clause.join(" OR ");

        let fa_query = format!(
            "SELECT
                submission.id,
                submission.url,
                submission.filename,
                submission.file_id,
                submission.hash,
                submission.hash_int,
                artist.name
            FROM
                submission
            JOIN artist
                ON artist.id = submission.artist_id
            WHERE
                {}",
            fa_where_clause.join(" OR ")
        );

        let e621_query = format!(
            "SELECT
                e621.id,
                e621.hash,
                e621.data->>'file_url' url,
                e621.data->>'md5' md5,
                sources.list sources,
                artists.list artists,
                (e621.data->>'md5') || '.' || (e621.data->>'file_ext') filename
            FROM
                e621,
                LATERAL (
                    SELECT array_agg(s) list
                    FROM jsonb_array_elements_text(data->'sources') s
                ) sources,
                LATERAL (
                    SELECT array_agg(s) list
                    FROM jsonb_array_elements_text(data->'artist') s
                ) artists
            WHERE
                {}",
            &hash_where_clause
        );

        let twitter_query = format!(
            "SELECT
                twitter_view.id,
                twitter_view.artists,
                twitter_view.url,
                twitter_view.hash
            FROM
                twitter_view
            WHERE
                {}",
            &hash_where_clause
        );

        let mut furaffinity = Box::pin(db.query::<str>(&*fa_query, &params).fuse());
        let mut e621 = Box::pin(db.query::<str>(&*e621_query, &params).fuse());
        let mut twitter = Box::pin(db.query::<str>(&*twitter_query, &params).fuse());

        #[allow(clippy::unnecessary_mut_passed)]
        loop {
            futures::select! {
                fa = furaffinity => {
                    let rows = fa.map(|rows| extract_fa_rows(rows, hash.as_deref()).into_iter().collect());
                    tx.send(rows).await.unwrap();
                }
                e = e621 => {
                    let rows = e.map(|rows| extract_e621_rows(rows, hash.as_deref()).into_iter().collect());
                    tx.send(rows).await.unwrap();
                }
                t = twitter => {
                    let rows = t.map(|rows| extract_twitter_rows(rows, hash.as_deref()).into_iter().collect());
                    tx.send(rows).await.unwrap();
                }
                complete => break,
            }
        }
    });

    rx
}
