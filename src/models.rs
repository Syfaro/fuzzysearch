use crate::types::*;

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
    db: DB<'_>,
    num: i64,
    distance: i64,
) -> Result<(Vec<tokio_postgres::Row>, Vec<tokio_postgres::Row>), tokio_postgres::Error> {
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&num, &distance];

    let fa = db.query(
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
                hash_int <@ ($1, $2)",
        &params,
    );

    let e621 = db.query(
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
                hash <@ ($1, $2)",
        &params,
    );

    let results = futures::future::join(fa, e621).await;
    Ok((results.0?, results.1?))
}
