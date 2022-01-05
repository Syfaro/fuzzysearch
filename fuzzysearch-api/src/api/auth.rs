use poem::Request;
use poem_openapi::{auth::ApiKey, SecurityScheme};

use crate::Pool;

/// Simple authentication using a static API key. Must be manually requested.
#[derive(SecurityScheme)]
#[oai(
    type = "api_key",
    key_name = "X-Api-Key",
    in = "header",
    checker = "api_checker"
)]
pub(crate) struct ApiKeyAuthorization(pub(crate) UserApiKey);

pub(crate) struct UserApiKey {
    pub(crate) id: i32,
    pub(crate) name: Option<String>,
    pub(crate) user_id: i32,
    pub(crate) name_limit: i16,
    pub(crate) image_limit: i16,
    pub(crate) hash_limit: i16,
}

#[tracing::instrument(skip(req, api_key))]
async fn api_checker(req: &Request, api_key: ApiKey) -> Option<UserApiKey> {
    let pool: &Pool = req.data().unwrap();

    let user_api_key = sqlx::query_file_as!(UserApiKey, "queries/lookup_api_key.sql", api_key.key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    if let Some(user_api_key) = user_api_key.as_ref() {
        tracing::debug!(
            api_key_id = user_api_key.id,
            app_name = user_api_key.name.as_deref().unwrap_or("unknown"),
            owner_id = %user_api_key.user_id,
            "found valid api key"
        );
    } else {
        tracing::warn!("request had invalid api key: {}", api_key.key);
    }

    user_api_key
}
