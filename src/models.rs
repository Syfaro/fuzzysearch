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
