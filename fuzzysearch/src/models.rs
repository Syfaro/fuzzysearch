use std::collections::HashSet;

use lazy_static::lazy_static;
use prometheus::{register_histogram, Histogram};
use tracing_futures::Instrument;

use crate::types::*;
use crate::{Pool, Tree};
use futures::TryStreamExt;
use fuzzysearch_common::types::{SearchResult, SiteInfo};

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
) -> Result<Vec<SearchResult>, sqlx::Error> {
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
) -> tokio::sync::mpsc::Receiver<Result<Vec<SearchResult>, sqlx::Error>> {
    let (tx, rx) = tokio::sync::mpsc::channel(50);

    tokio::spawn(
        async move {
            let db = pool;

            for query_hash in hashes {
                tracing::trace!(query_hash, "Evaluating hash");

                let mut seen: HashSet<[u8; 8]> = HashSet::new();

                let _timer = IMAGE_LOOKUP_DURATION.start_timer();

                let node = crate::Node::query(query_hash.to_be_bytes());
                let lock = tree.read().await;
                let items = lock.find(&node, distance as u64);

                for (dist, item) in items {
                    if seen.contains(&item.0) {
                        tracing::trace!("Already searched for hash");
                        continue;
                    }
                    seen.insert(item.0);

                    let _timer = IMAGE_QUERY_DURATION.start_timer();

                    tracing::debug!(num = item.num(), "Searching database for hash in tree");

                    let mut row = sqlx::query!(
                        "SELECT
                            'FurAffinity' site,
                            submission.id,
                            submission.hash_int hash,
                            submission.url,
                            submission.filename,
                            ARRAY(SELECT artist.name) artists,
                            submission.file_id,
                            null sources,
                            submission.rating
                        FROM submission
                        JOIN artist ON submission.artist_id = artist.id
                        WHERE hash_int <@ ($1, 0)
                        UNION
                        SELECT
                            'e621' site,
                            e621.id,
                            e621.hash,
                            e621.data->'file'->>'url' url,
                            (e621.data->'file'->>'md5') || '.' || (e621.data->'file'->>'ext') filename,
                            ARRAY(SELECT jsonb_array_elements_text(e621.data->'tags'->'artist')) artists,
                            null file_id,
                            ARRAY(SELECT jsonb_array_elements_text(e621.data->'sources')) sources,
                            e621.data->>'rating' rating
                        FROM e621
                        WHERE hash <@ ($1, 0)
                        UNION
                        SELECT
                            'Weasyl' site,
                            weasyl.id,
                            weasyl.hash,
                            weasyl.data->>'link' url,
                            null filename,
                            ARRAY(SELECT weasyl.data->>'owner_login') artists,
                            null file_id,
                            null sources,
                            weasyl.data->>'rating' rating
                        FROM weasyl
                        WHERE hash <@ ($1, 0)
                        UNION
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
                            END rating
                        FROM tweet_media
                        JOIN tweet ON tweet_media.tweet_id = tweet.id
                        WHERE hash <@ ($1, 0)",
                        &item.num()
                    )
                    .map(|row| {
                        let site_info = match row.site.as_deref() {
                            Some("FurAffinity") => SiteInfo::FurAffinity { file_id: row.file_id.unwrap_or(-1) },
                            Some("e621") => SiteInfo::E621 { sources: row.sources },
                            Some("Twitter") => SiteInfo::Twitter,
                            Some("Weasyl") => SiteInfo::Weasyl,
                            _ => panic!("Got unknown site"),
                        };

                        let file = SearchResult {
                            site_id: row.id.unwrap_or_default(),
                            site_info: Some(site_info),
                            rating: row.rating.and_then(|rating| rating.parse().ok()),
                            site_id_str: row.id.unwrap_or_default().to_string(),
                            url: row.url.unwrap_or_default(),
                            hash: row.hash,
                            distance: Some(dist),
                            artists: row.artists,
                            filename: row.filename.unwrap_or_default(),
                            searched_hash: Some(query_hash),
                        };

                        vec![file]
                    })
                    .fetch(&db);

                    while let Some(row) = row.try_next().await.ok().flatten() {
                        tx.send(Ok(row)).await.unwrap();
                    }
                }
            }
        }
        .in_current_span(),
    );

    rx
}
