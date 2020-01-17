use crate::models::DB;
use crate::types::*;
use log::debug;

#[macro_export]
macro_rules! rate_limit {
    ($api_key:expr, $db:expr, $limit:tt, $group:expr) => {
        rate_limit!($api_key, $db, $limit, $group, 1)
    };

    ($api_key:expr, $db:expr, $limit:tt, $group:expr, $incr_by:expr) => {
        let api_key = crate::models::lookup_api_key($api_key, $db)
            .await
            .ok_or_else(|| warp::reject::custom(Error::ApiKey))?;

        let rate_limit =
            crate::utils::update_rate_limit($db, api_key.id, api_key.$limit, $group, $incr_by)
                .await
                .map_err(crate::handlers::map_postgres_err)?;

        if rate_limit == crate::types::RateLimit::Limited {
            return Err(warp::reject::custom(Error::RateLimit));
        }
    };
}

/// Increment the rate limit for a group.
///
/// We need to specify the ID of the API key to increment, the key's limit for
/// the specified group, the name of the group we're incrementing, and the
/// amount to increment for this request. This should remain as 1 except for
/// joined requests.
pub async fn update_rate_limit(
    db: DB<'_>,
    key_id: i32,
    key_group_limit: i16,
    group_name: &'static str,
    incr_by: i16,
) -> Result<RateLimit, tokio_postgres::Error> {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp();
    let time_window = timestamp - (timestamp % 60);

    debug!(
        "Incrementing rate limit for: {}-{} by {}",
        key_id, group_name, incr_by
    );

    let rows = db
        .query(
            "INSERT INTO
                rate_limit (api_key_id, time_window, group_name, count)
            VALUES
                ($1, $2, $3, $4)
            ON CONFLICT ON CONSTRAINT unique_window
                DO UPDATE set count = rate_limit.count + $4
            RETURNING rate_limit.count",
            &[&key_id, &time_window, &group_name, &incr_by],
        )
        .await?;

    let count: i16 = rows[0].get(0);

    if count > key_group_limit {
        Ok(RateLimit::Limited)
    } else {
        Ok(RateLimit::Available(count))
    }
}

pub fn extract_fa_rows<'a>(
    rows: Vec<tokio_postgres::Row>,
    hash: Option<&'a [u8]>,
) -> impl IntoIterator<Item = File> + 'a {
    rows.into_iter().map(move |row| {
        let dbbytes: Vec<u8> = row.get("hash");

        File {
            id: row.get("id"),
            url: row.get("url"),
            filename: row.get("filename"),
            hash: row.get("hash_int"),
            distance: hash
                .map(|hash| hamming::distance_fast(&dbbytes, &hash).ok())
                .flatten(),
            site_info: Some(SiteInfo::FurAffinity(FurAffinityFile {
                file_id: row.get("file_id"),
            })),
            artists: row.get::<&str, Option<String>>("name").map(|row| vec![row]),
        }
    })
}

pub fn extract_e621_rows<'a>(
    rows: Vec<tokio_postgres::Row>,
    hash: Option<&'a [u8]>,
) -> impl IntoIterator<Item = File> + 'a {
    rows.into_iter().map(move |row| {
        let dbhash: i64 = row.get("hash");
        let dbbytes = dbhash.to_be_bytes();

        File {
            id: row.get("id"),
            url: row.get("url"),
            hash: Some(dbhash),
            distance: hash
                .map(|hash| hamming::distance_fast(&dbbytes, &hash).ok())
                .flatten(),
            site_info: Some(SiteInfo::E621(E621File {
                file_md5: row.get("md5"),
                sources: row.get("sources"),
            })),
            artists: row.get("artists"),
            filename: row.get("filename"),
        }
    })
}
