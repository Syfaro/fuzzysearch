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
        let rating = match s {
            "g" | "s" | "general" => Self::General,
            "m" | "q" | "mature" => Self::Mature,
            "a" | "e" | "adult" => Self::Adult,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebHookData {
    pub site: Site,
    pub site_id: i32,
    pub artist: String,
    pub file_url: String,
    #[serde(with = "b64")]
    pub file_sha256: Option<Vec<u8>>,
}

mod b64 {
    use serde::Deserialize;

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match bytes {
            Some(bytes) => serializer.serialize_str(&base64::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = <Option<String>>::deserialize(deserializer)?
            .map(base64::decode)
            .transpose()
            .map_err(serde::de::Error::custom)?;

        Ok(val)
    }
}
