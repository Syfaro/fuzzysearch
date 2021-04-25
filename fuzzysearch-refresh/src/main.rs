use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use furaffinity_rs::FurAffinity;
use tracing_unwrap::ResultExt;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("missing data: {0}")]
    MissingData(&'static str),
    #[error("furaffinity error")]
    FurAffinity(furaffinity_rs::Error),
    #[error("faktory error")]
    Faktory,
}

static FURAFFINITY_QUEUE: &str = "fuzzysearch_refresh_furaffinity";

type Producer = Arc<Mutex<faktory::Producer<TcpStream>>>;
type Db = sqlx::Pool<sqlx::Postgres>;

fn main() {
    fuzzysearch_common::init_logger();

    tracing::info!("initializing");

    let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());

    let mut faktory = faktory::ConsumerBuilder::default();
    faktory.workers(2);

    let p = Arc::new(Mutex::new(faktory::Producer::connect(None).unwrap_or_log()));

    let pool = rt
        .block_on(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&std::env::var("DATABASE_URL").unwrap_or_log()),
        )
        .unwrap_or_log();

    let (cookie_a, cookie_b) = (
        std::env::var("FA_A").unwrap_or_log(),
        std::env::var("FA_B").unwrap_or_log(),
    );
    let user_agent = std::env::var("USER_AGENT").unwrap_or_log();
    let client = reqwest::Client::new();
    let fa = Arc::new(FurAffinity::new(
        cookie_a,
        cookie_b,
        user_agent,
        Some(client),
    ));

    rt.spawn(poll_fa_online(fa.clone(), p.clone()));

    let rt_clone = rt.clone();
    let pool_clone = pool.clone();
    faktory.register("furaffinity_load", move |job| -> Result<(), Error> {
        use std::convert::TryFrom;

        let id = job
            .args()
            .iter()
            .next()
            .ok_or(Error::MissingData("submission id"))?
            .as_i64()
            .ok_or(Error::MissingData("submission id"))?;

        let id = i32::try_from(id).map_err(|_| Error::MissingData("invalid id"))?;

        let last_updated = rt_clone
            .block_on(
                sqlx::query_scalar!("SELECT updated_at FROM submission WHERE id = $1", id)
                    .fetch_optional(&pool_clone),
            )?
            .flatten();

        if let Some(last_updated) = last_updated {
            let diff = last_updated.signed_duration_since(chrono::Utc::now());
            if diff.num_days() < 30 {
                tracing::warn!("attempted to check recent submission, skipping");
                return Ok(());
            }
        }

        let sub = rt_clone
            .block_on(fa.get_submission(id))
            .map_err(Error::FurAffinity)?;

        tracing::debug!("loaded furaffinity submission");

        rt_clone.block_on(update_furaffinity_submission(
            pool_clone.clone(),
            fa.clone(),
            id,
            sub,
        ))?;

        Ok(())
    });

    faktory.register(
        "furaffinity_calculate_missing",
        move |job| -> Result<(), Error> {
            use std::collections::HashSet;

            let batch_size = job
                .args()
                .iter()
                .next()
                .map(|arg| arg.as_i64())
                .flatten()
                .unwrap_or(1_000);

            tracing::debug!(batch_size, "calculating missing submissions");

            let known_ids: HashSet<_> = rt
                .block_on(sqlx::query_scalar!("SELECT id FROM submission").fetch_all(&pool))?
                .into_iter()
                .collect();
            let all_ids: HashSet<_> = (1..=*known_ids.iter().max().unwrap_or(&1)).collect();
            let missing_ids: Vec<_> = all_ids
                .difference(&known_ids)
                .take(batch_size as usize)
                .collect();

            tracing::info!(
                missing = missing_ids.len(),
                "enqueueing batch of missing submissions"
            );

            let mut p = p.lock().unwrap_or_log();

            for id in missing_ids {
                let job =
                    faktory::Job::new("furaffinity_load", vec![*id]).on_queue(FURAFFINITY_QUEUE);
                p.enqueue(job).map_err(|_err| Error::Faktory)?;
            }

            Ok(())
        },
    );

    let faktory = faktory.connect(None).unwrap_or_log();
    tracing::info!("starting to run queues");
    faktory.run_to_completion(&["fuzzysearch_refresh", FURAFFINITY_QUEUE]);
}

/// Check the number of users on FurAffinity every minute and control if queues
/// are allowed to run.
async fn poll_fa_online(fa: Arc<FurAffinity>, p: Producer) {
    use futures::StreamExt;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use tokio::time::interval;
    use tokio_stream::wrappers::IntervalStream;

    let max_online = std::env::var("MAX_ONLINE")
        .ok()
        .and_then(|num| num.parse().ok())
        .unwrap_or(10_000);

    tracing::info!(max_online, "got max fa users online before pause");

    // Ensure initial state of the queue being enabled.
    {
        let p = p.clone();
        tokio::task::spawn_blocking(move || {
            let mut p = p.lock().unwrap_or_log();
            p.queue_resume(&[FURAFFINITY_QUEUE]).unwrap_or_log();
        })
        .await
        .expect_or_log("could not set initial queue state");
    }

    let queue_state = AtomicBool::new(true);

    IntervalStream::new(interval(Duration::from_secs(300)))
        .for_each(|_| {
            let p = p.clone();

            async {
                let continue_queue = match fa.latest_id().await {
                    Ok((_latest_id, online)) => {
                        tracing::debug!(registered = online.registered, "got updated fa online");
                        online.registered < max_online
                    }
                    Err(err) => {
                        tracing::error!("unable to get fa online: {:?}", err);
                        false
                    }
                };

                if queue_state.load(Ordering::SeqCst) == continue_queue {
                    tracing::trace!("fa queue was already in correct state");
                    return;
                }

                tracing::info!(continue_queue, "updating fa queue state");

                let result = tokio::task::spawn_blocking(move || {
                    let mut p = p.lock().unwrap_or_log();

                    if continue_queue {
                        p.queue_resume(&[FURAFFINITY_QUEUE])
                    } else {
                        p.queue_pause(&[FURAFFINITY_QUEUE])
                    }
                })
                .await;

                match result {
                    Err(err) => tracing::error!("unable to join queue change: {:?}", err),
                    Ok(Err(err)) => tracing::error!("unable to change fa queue state: {:?}", err),
                    _ => queue_state.store(continue_queue, Ordering::SeqCst),
                }
            }
        })
        .await;
}

async fn get_furaffinity_artist(db: &Db, artist: &str) -> Result<i32, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar!("SELECT id FROM artist WHERE name = $1", artist)
        .fetch_optional(db)
        .await?
    {
        return Ok(id);
    }

    sqlx::query_scalar!("INSERT INTO artist (name) VALUES ($1) RETURNING id", artist)
        .fetch_one(db)
        .await
}

async fn get_furaffinity_tag(db: &Db, tag: &str) -> Result<i32, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar!("SELECT id FROM tag WHERE name = $1", tag)
        .fetch_optional(db)
        .await?
    {
        return Ok(id);
    }

    sqlx::query_scalar!("INSERT INTO tag (name) VALUES ($1) RETURNING id", tag)
        .fetch_one(db)
        .await
}

async fn associate_furaffinity_tag(db: &Db, id: i32, tag_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO tag_to_post (tag_id, post_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        tag_id,
        id
    )
    .execute(db)
    .await
    .map(|_| ())
}

async fn update_furaffinity_submission(
    db: Db,
    fa: Arc<FurAffinity>,
    id: i32,
    sub: Option<furaffinity_rs::Submission>,
) -> Result<(), Error> {
    let sub = match sub {
        Some(sub) => sub,
        None => {
            tracing::info!(id, "furaffinity submission did not exist");
            sqlx::query!("INSERT INTO submission (id, updated_at, deleted) VALUES ($1, current_timestamp, true) ON CONFLICT (id) DO UPDATE SET deleted = true", id).execute(&db).await?;
            return Ok(());
        }
    };

    let sub = fa.calc_image_hash(sub).await.map_err(Error::FurAffinity)?;

    let artist_id = get_furaffinity_artist(&db, &sub.artist).await?;

    let mut tag_ids = Vec::with_capacity(sub.tags.len());
    for tag in &sub.tags {
        tag_ids.push(get_furaffinity_tag(&db, tag).await?);
    }

    let hash = sub.hash.clone();
    let url = sub.content.url();

    let size = sub.file_size.map(|size| size as i32);

    sqlx::query!(
        "INSERT INTO submission
            (id, artist_id, url, filename, hash, rating, posted_at, description, hash_int, file_id, file_size, file_sha256, updated_at) VALUES
            ($1, $2, $3, $4, decode($5, 'base64'), $6, $7, $8, $9, CASE WHEN isnumeric(split_part($4, '.', 1)) THEN split_part($4, '.', 1)::int ELSE null END, $10, $11, current_timestamp)
            ON CONFLICT (id) DO UPDATE SET url = $3, filename = $4, hash = decode($5, 'base64'), rating = $6, description = $8, hash_int = $9, file_id = CASE WHEN isnumeric(split_part($4, '.', 1)) THEN split_part($4, '.', 1)::int ELSE null END, file_size = $10, file_sha256 = $11, updated_at = current_timestamp",
        sub.id, artist_id, url, sub.filename, hash, sub.rating.serialize(), sub.posted_at, sub.description, sub.hash_num, size, sub.file_sha256,
    )
    .execute(&db).await?;

    for tag_id in tag_ids {
        associate_furaffinity_tag(&db, id, tag_id).await?;
    }

    Ok(())
}
