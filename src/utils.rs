use crate::types::*;

#[macro_export]
macro_rules! rate_limit {
    ($api_key:expr, $db:expr, $limit:tt, $group:expr) => {
        rate_limit!($api_key, $db, $limit, $group, 1)
    };

    ($api_key:expr, $db:expr, $limit:tt, $group:expr, $incr_by:expr) => {{
        let api_key = match crate::models::lookup_api_key($api_key, $db).await {
            Some(api_key) => api_key,
            None => return Ok(Box::new(Error::ApiKey)),
        };

        let rate_limit = match crate::utils::update_rate_limit(
            $db,
            api_key.id,
            api_key.$limit,
            $group,
            $incr_by,
        )
        .await
        {
            Ok(rate_limit) => rate_limit,
            Err(err) => return Ok(Box::new(Error::Postgres(err))),
        };

        match rate_limit {
            crate::types::RateLimit::Limited => return Ok(Box::new(Error::RateLimit)),
            crate::types::RateLimit::Available(count) => count,
        }
    }};
}

#[macro_export]
macro_rules! early_return {
    ($val:expr) => {
        match $val {
            Ok(val) => val,
            Err(err) => return Ok(Box::new(Error::from(err))),
        }
    };
}

/// Increment the rate limit for a group.
///
/// We need to specify the ID of the API key to increment, the key's limit for
/// the specified group, the name of the group we're incrementing, and the
/// amount to increment for this request. This should remain as 1 except for
/// joined requests.
#[tracing::instrument(skip(db))]
pub async fn update_rate_limit(
    db: &sqlx::PgPool,
    key_id: i32,
    key_group_limit: i16,
    group_name: &'static str,
    incr_by: i16,
) -> Result<RateLimit, sqlx::Error> {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp();
    let time_window = timestamp - (timestamp % 60);

    let count: i16 = sqlx::query_scalar!(
        "INSERT INTO
            rate_limit (api_key_id, time_window, group_name, count)
        VALUES
            ($1, $2, $3, $4)
        ON CONFLICT ON CONSTRAINT unique_window
            DO UPDATE set count = rate_limit.count + $4
        RETURNING rate_limit.count",
        key_id,
        time_window,
        group_name,
        incr_by
    )
    .fetch_one(db)
    .await?;

    if count > key_group_limit {
        Ok(RateLimit::Limited)
    } else {
        Ok(RateLimit::Available((
            key_group_limit - count,
            key_group_limit,
        )))
    }
}
