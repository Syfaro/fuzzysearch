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

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FurAffinity => write!(f, "FurAffinity"),
            Self::E621 => write!(f, "e621"),
            Self::Weasyl => write!(f, "Weasyl"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebHookData {
    pub site: Site,
    pub site_id: i32,
    pub artist: String,
    pub file_url: String,
    #[serde(with = "b64_vec")]
    pub file_sha256: Option<Vec<u8>>,
    #[serde(with = "b64_u8")]
    pub hash: Option<[u8; 8]>,
}

mod b64_vec {
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

mod b64_u8 {
    use std::convert::TryInto;

    use serde::Deserialize;

    pub fn serialize<S, const N: usize>(
        bytes: &Option<[u8; N]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match bytes {
            Some(bytes) => serializer.serialize_str(&base64::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<Option<[u8; N]>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = <Option<String>>::deserialize(deserializer)?
            .map(base64::decode)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .map(|bytes| bytes.try_into())
            .transpose()
            .map_err(|_err| "value did not have correct number of bytes")
            .map_err(serde::de::Error::custom)?;

        Ok(val)
    }
}
