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

pub struct ImageQueryResults {
    pub furaffinity: Vec<tokio_postgres::Row>,
    pub e621: Vec<tokio_postgres::Row>,
    pub twitter: Vec<tokio_postgres::Row>,
}

impl ImageQueryResults {
    #[inline]
    pub fn len(&self) -> usize {
        self.furaffinity.len() + self.e621.len() + self.twitter.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub async fn image_query(
    db: DB<'_>,
    hashes: Vec<i64>,
    distance: i64,
) -> Result<ImageQueryResults, tokio_postgres::Error> {
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

    let furaffinity = db.query::<str>(&*fa_query, &params);
    let e621 = db.query::<str>(&*e621_query, &params);
    let twitter = db.query::<str>(&*twitter_query, &params);

    let results = futures::future::join3(furaffinity, e621, twitter).await;
    Ok(ImageQueryResults {
        furaffinity: results.0?,
        e621: results.1?,
        twitter: results.2?,
    })
}
