use serde::{Deserialize, Serialize};

/// An API key representation from the database.alloc
///
/// May contain information about the owner, always has rate limit information.
/// Limits are the number of requests allowed per minute.
#[derive(Debug)]
pub struct ApiKey {
    pub id: i32,
    pub name: Option<String>,
    pub owner_email: Option<String>,
    pub name_limit: i16,
    pub image_limit: i16,
}

/// The status of an API key's rate limit.
#[derive(Debug, PartialEq)]
pub enum RateLimit {
    /// This key is limited, we should deny the request.
    Limited,
    /// This key is available, contains the number of requests made.
    Available(i16),
}

/// A general type for every file.
#[derive(Debug, Default, Serialize)]
pub struct File {
    pub id: i32,

    pub site_id: i64,
    pub site_id_str: String,

    pub url: String,
    pub filename: String,
    pub artists: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub site_info: Option<SiteInfo>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "site", content = "site_info")]
pub enum SiteInfo {
    FurAffinity(FurAffinityFile),
    #[serde(rename = "e621")]
    E621(E621File),
    Twitter,
}

/// Information about a file hosted on FurAffinity.
#[derive(Debug, Serialize)]
pub struct FurAffinityFile {
    pub file_id: i32,
}

/// Information about a file hosted on e621.
#[derive(Debug, Serialize)]
pub struct E621File {
    pub sources: Option<Vec<String>>,
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
    pub matches: Vec<File>,
}

#[derive(Serialize)]
pub struct ErrorMessage {
    pub code: u16,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct HashSearchOpts {
    pub hashes: String,
}
