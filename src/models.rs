use crate::types::*;
use crate::{Pool, Tree};
use lazy_static::lazy_static;
use prometheus::{register_histogram, Histogram};
use tracing_futures::Instrument;

lazy_static! {
    static ref IMAGE_LOOKUP_DURATION: Histogram = register_histogram!(
        "fuzzysearch_api_image_lookup_seconds",
        "Duration to perform an image lookup"
    )
    .unwrap();
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

#[tracing::instrument(skip(pool, tree))]
pub async fn image_query(
    pool: Pool,
    tree: Tree,
    hashes: Vec<i64>,
    distance: i64,
    hash: Option<Vec<u8>>,
) -> Result<Vec<File>, sqlx::Error> {
    let mut results = image_query_sync(pool, tree, hashes, distance, hash);
    let mut matches = Vec::new();

    while let Some(r) = results.recv().await {
        matches.extend(r?);
    }

    Ok(matches)
}

#[tracing::instrument(skip(pool, tree))]
pub fn image_query_sync(
    pool: Pool,
    tree: Tree,
    hashes: Vec<i64>,
    distance: i64,
    hash: Option<Vec<u8>>,
) -> tokio::sync::mpsc::Receiver<Result<Vec<File>, sqlx::Error>> {
    let (tx, rx) = tokio::sync::mpsc::channel(50);

    tokio::spawn(async move {
        let db = pool;

        for query_hash in hashes {
            let mut seen = std::collections::HashSet::new();

            let _timer = IMAGE_LOOKUP_DURATION.start_timer();

            let node = crate::Node::query(query_hash.to_be_bytes());
            let lock = tree.read().await;
            let items = lock.find(&node, distance as u64);

            for (dist, item) in items {
                if seen.contains(&item.id) {
                    continue;
                }
                seen.insert(item.id);

                let _timer = IMAGE_QUERY_DURATION.start_timer();

                let row = sqlx::query!("SELECT
                        hashes.id,
                        hashes.hash,
                        hashes.furaffinity_id,
                        hashes.e621_id,
                        hashes.twitter_id,
                    CASE
                        WHEN furaffinity_id IS NOT NULL THEN (f.url)
                        WHEN e621_id IS NOT NULL THEN (e.data->'file'->>'url')
                        WHEN twitter_id IS NOT NULL THEN (tm.url)
                    END url,
                    CASE
                        WHEN furaffinity_id IS NOT NULL THEN (f.filename)
                        WHEN e621_id IS NOT NULL THEN ((e.data->'file'->>'md5') || '.' || (e.data->'file'->>'ext'))
                        WHEN twitter_id IS NOT NULL THEN (SELECT split_part(split_part(tm.url, '/', 5), ':', 1))
                    END filename,
                    CASE
                        WHEN furaffinity_id IS NOT NULL THEN (ARRAY(SELECT f.name))
                        WHEN e621_id IS NOT NULL THEN ARRAY(SELECT jsonb_array_elements_text(e.data->'tags'->'artist'))
                        WHEN twitter_id IS NOT NULL THEN ARRAY(SELECT tw.data->'user'->>'screen_name')
                    END artists,
                    CASE
                        WHEN furaffinity_id IS NOT NULL THEN (f.file_id)
                    END file_id,
                    CASE
                        WHEN e621_id IS NOT NULL THEN ARRAY(SELECT jsonb_array_elements_text(e.data->'sources'))
                    END sources,
                    CASE
                        WHEN furaffinity_id IS NOT NULL THEN (f.rating)
                        WHEN e621_id IS NOT NULL THEN (e.data->>'rating')
                        WHEN twitter_id IS NOT NULL THEN
                            CASE
                                WHEN (tw.data->'possibly_sensitive')::boolean IS true THEN 'adult'
                                WHEN (tw.data->'possibly_sensitive')::boolean IS false THEN 'general'
                            END
                    END rating
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
                        tweet_media.hash <@ (hashes.hash, 0)
                    LIMIT 1
                ) tm ON hashes.twitter_id IS NOT NULL
                WHERE hashes.id = $1", item.id).map(|row| {
                    let (site_id, site_info) = if let Some(fa_id) = row.furaffinity_id {
                        (
                            fa_id as i64,
                            Some(SiteInfo::FurAffinity(FurAffinityFile {
                                file_id: row.file_id.unwrap(),
                            }))
                        )
                    } else if let Some(e621_id) = row.e621_id {
                        (
                            e621_id as i64,
                            Some(SiteInfo::E621(E621File {
                                sources: row.sources,
                            }))
                        )
                    } else if let Some(twitter_id) = row.twitter_id {
                        (twitter_id, Some(SiteInfo::Twitter))
                    } else {
                        (-1, None)
                    };

                    let file = File {
                        id: row.id,
                        site_id,
                        site_info,
                        rating: row.rating.and_then(|rating| rating.parse().ok()),
                        site_id_str: site_id.to_string(),
                        url: row.url.unwrap_or_default(),
                        hash: Some(row.hash),
                        distance: Some(dist),
                        artists: row.artists,
                        filename: row.filename.unwrap_or_default(),
                        searched_hash: Some(query_hash),
                    };

                    vec![file]
                }).fetch_one(&db).await;

                tx.send(row).await.unwrap();
            }
        }
    }.in_current_span());

    rx
}
