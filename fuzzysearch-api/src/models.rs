use lazy_static::lazy_static;
use prometheus::{register_histogram, Histogram};

use crate::types::*;
use crate::Pool;
use fuzzysearch_common::types::{SearchResult, SiteInfo};

lazy_static! {
    static ref IMAGE_QUERY_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_query_seconds",
        "Duration to perform a single image lookup query"
    )
    .unwrap();
}

#[tracing::instrument(skip(db))]
pub async fn lookup_api_key(key: &str, db: &sqlx::PgPool) -> Option<ApiKey> {
    sqlx::query_as!(
        ApiKey,
        "SELECT
            api_key.id,
            api_key.name_limit,
            api_key.image_limit,
            api_key.hash_limit,
            api_key.name,
            account.email owner_email
        FROM
            api_key
        JOIN account
            ON account.id = api_key.user_id
        WHERE
            api_key.key = $1
    ",
        key
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[derive(serde::Serialize)]
struct HashSearch {
    searched_hash: i64,
    found_hash: i64,
    distance: u64,
}

#[tracing::instrument(skip(pool, bkapi))]
pub async fn image_query(
    pool: Pool,
    bkapi: bkapi_client::BKApiClient,
    hashes: Vec<i64>,
    distance: i64,
) -> Result<Vec<SearchResult>, sqlx::Error> {
    let found_hashes: Vec<HashSearch> = bkapi
        .search_many(&hashes, distance as u64)
        .await
        .unwrap()
        .into_iter()
        .flat_map(|results| {
            results
                .hashes
                .iter()
                .map(|hash| HashSearch {
                    searched_hash: results.hash,
                    found_hash: hash.hash,
                    distance: hash.distance,
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let timer = IMAGE_QUERY_DURATION.start_timer();
    let matches = sqlx::query!(
        r#"WITH hashes AS (
            SELECT * FROM jsonb_to_recordset($1::jsonb)
                AS hashes(searched_hash bigint, found_hash bigint, distance bigint)
        )
        SELECT
            'FurAffinity' site,
            submission.id,
            submission.hash_int hash,
            submission.url,
            submission.filename,
            ARRAY(SELECT artist.name) artists,
            submission.file_id,
            null sources,
            submission.rating,
            submission.posted_at,
            hashes.searched_hash,
            hashes.distance
        FROM hashes
        JOIN submission ON hashes.found_hash = submission.hash_int
        JOIN artist ON submission.artist_id = artist.id
        WHERE hash_int IN (SELECT hashes.found_hash)
        UNION ALL
        SELECT
            'e621' site,
            e621.id,
            e621.hash,
            e621.data->'file'->>'url' url,
            (e621.data->'file'->>'md5') || '.' || (e621.data->'file'->>'ext') filename,
            ARRAY(SELECT jsonb_array_elements_text(e621.data->'tags'->'artist')) artists,
            null file_id,
            ARRAY(SELECT jsonb_array_elements_text(e621.data->'sources')) sources,
            e621.data->>'rating' rating,
            to_timestamp(data->>'created_at', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') posted_at,
            hashes.searched_hash,
            hashes.distance
        FROM hashes
        JOIN e621 ON hashes.found_hash = e621.hash
        WHERE e621.hash IN (SELECT hashes.found_hash)
        UNION ALL
        SELECT
            'Weasyl' site,
            weasyl.id,
            weasyl.hash,
            weasyl.data->>'link' url,
            null filename,
            ARRAY(SELECT weasyl.data->>'owner_login') artists,
            null file_id,
            null sources,
            weasyl.data->>'rating' rating,
            to_timestamp(data->>'posted_at', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') posted_at,
            hashes.searched_hash,
            hashes.distance
        FROM hashes
        JOIN weasyl ON hashes.found_hash = weasyl.hash
        WHERE weasyl.hash IN (SELECT hashes.found_hash)
        UNION ALL
        SELECT
            'Twitter' site,
            tweet.id,
            tweet_media.hash,
            tweet_media.url,
            null filename,
            ARRAY(SELECT tweet.data->'user'->>'screen_name') artists,
            null file_id,
            null sources,
            CASE
                WHEN (tweet.data->'possibly_sensitive')::boolean IS true THEN 'adult'
                WHEN (tweet.data->'possibly_sensitive')::boolean IS false THEN 'general'
            END rating,
            to_timestamp(tweet.data->>'created_at', 'DY Mon DD HH24:MI:SS +0000 YYYY') posted_at,
            hashes.searched_hash,
            hashes.distance
        FROM hashes
        JOIN tweet_media ON hashes.found_hash = tweet_media.hash
        JOIN tweet ON tweet_media.tweet_id = tweet.id
        WHERE tweet_media.hash IN (SELECT hashes.found_hash)"#,
        serde_json::to_value(&found_hashes).unwrap()
    )
    .map(|row| {
        use std::convert::TryFrom;

        let site_info = match row.site.as_deref() {
            Some("FurAffinity") => SiteInfo::FurAffinity {
                file_id: row.file_id.unwrap_or(-1),
            },
            Some("e621") => SiteInfo::E621 {
                sources: row.sources,
            },
            Some("Twitter") => SiteInfo::Twitter,
            Some("Weasyl") => SiteInfo::Weasyl,
            _ => panic!("Got unknown site"),
        };

        SearchResult {
            site_id: row.id.unwrap_or_default(),
            site_info: Some(site_info),
            rating: row.rating.and_then(|rating| rating.parse().ok()),
            site_id_str: row.id.unwrap_or_default().to_string(),
            url: row.url.unwrap_or_default(),
            posted_at: row.posted_at,
            hash: row.hash,
            distance: row
                .distance
                .map(|distance| u64::try_from(distance).ok())
                .flatten(),
            artists: row.artists,
            filename: row.filename.unwrap_or_default(),
            searched_hash: row.searched_hash,
        }
    })
    .fetch_all(&pool)
    .await?;
    timer.stop_and_record();

    Ok(matches)
}
