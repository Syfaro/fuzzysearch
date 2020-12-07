use serde::Serialize;

/// A general type for every result in a search.
#[derive(Debug, Default, Serialize)]
pub struct SearchResult {
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub searched_hash: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "site", content = "site_info")]
pub enum SiteInfo {
    FurAffinity {
        file_id: i32,
    },
    #[serde(rename = "e621")]
    E621 {
        sources: Option<Vec<String>>,
    },
    Twitter,
}
