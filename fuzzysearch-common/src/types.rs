use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    General,
    Mature,
    Adult,
}

impl std::str::FromStr for Rating {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Each site has their own system of content ratings...
        let rating = match s {
            "g" | "s" | "general" => Self::General,
            "m" | "q" | "mature" => Self::Mature,
            "a" | "e" | "adult" | "explicit" => Self::Adult,
            _ => return Err("unknown rating"),
        };

        Ok(rating)
    }
}

/// A general type for every result in a search.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SearchResult {
    pub site_id: i64,
    pub site_id_str: String,

    pub url: String,
    pub filename: String,
    pub artists: Option<Vec<String>>,
    pub rating: Option<Rating>,
    pub posted_at: Option<chrono::DateTime<chrono::Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    Weasyl,
}

#[derive(Copy, Clone, Deserialize, Serialize, Debug)]
pub enum Site {
    FurAffinity,
    E621,
    Weasyl,
    Twitter,
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FurAffinity => write!(f, "FurAffinity"),
            Self::E621 => write!(f, "e621"),
            Self::Weasyl => write!(f, "Weasyl"),
            Self::Twitter => write!(f, "Twitter"),
        }
    }
}
