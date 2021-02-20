use serde::{Deserialize, Serialize};

use fuzzysearch_common::types::SearchResult;

/// An API key representation from the database.alloc
///
/// May contain information about the owner, always has rate limit information.
/// Limits are the number of requests allowed per minute.
#[derive(Debug)]
pub struct ApiKey {
    pub id: i32,
    pub name: Option<String>,
    pub owner_email: String,
    pub name_limit: i16,
    pub image_limit: i16,
    pub hash_limit: i16,
}

/// The status of an API key's rate limit.
#[derive(Debug, PartialEq)]
pub enum RateLimit {
    /// This key is limited, we should deny the request.
    Limited,
    /// This key is available, contains the number of requests made.
    Available((i16, i16)),
}

#[derive(Debug, Deserialize)]
pub struct FileSearchOpts {
    pub id: Option<i32>,
    pub name: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImageSearchOpts {
    #[serde(rename = "type")]
    pub search_type: Option<ImageSearchType>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImageSearchType {
    Close,
    Exact,
    Force,
}

#[derive(Debug, Serialize)]
pub struct ImageSimilarity {
    pub hash: i64,
    pub matches: Vec<SearchResult>,
}

#[derive(Serialize)]
pub struct ErrorMessage {
    pub code: u16,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct HashSearchOpts {
    pub hashes: String,
    pub distance: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct HandleOpts {
    pub twitter: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UrlSearchOpts {
    pub url: String,
}
