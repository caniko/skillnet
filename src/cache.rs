#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub stamps: BTreeMap<String, ScopeStamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeStamp {
    pub last_pulled_at: SystemTime,
    #[serde(with = "u128_string")]
    pub live_source_max_mtime_nanos: u128,
    pub mirror_content_hash: String,
}

pub fn cache_path(mirror_root: &Utf8Path) -> Utf8PathBuf {
    mirror_root.join(".skillnet").join("cache.toml")
}

pub fn load(mirror_root: &Utf8Path) -> Cache {
    let path = cache_path(mirror_root);
    fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(mirror_root: &Utf8Path, cache: &Cache) -> Result<()> {
    let path = cache_path(mirror_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(cache)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn live_source_max_mtime(sources: &[Utf8PathBuf]) -> Result<u128> {
    let mut newest = 0;
    for source in sources {
        if !source.is_dir() {
            continue;
        }

        for entry in WalkDir::new(source)
            .follow_links(false)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| !is_dot_entry(entry))
        {
            let entry = entry?;
            if entry.file_type().is_file() || entry.file_type().is_symlink() {
                let modified = entry
                    .metadata()?
                    .modified()
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                let nanos = modified
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                newest = newest.max(nanos);
            }
        }
    }
    Ok(newest)
}

pub fn is_stale(stamp: &ScopeStamp, live_mtime: u128) -> bool {
    live_mtime > stamp.live_source_max_mtime_nanos
}

fn is_dot_entry(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.'))
}

mod u128_string {
    use std::fmt;

    use serde::{de, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct U128Visitor;

        impl de::Visitor<'_> for U128Visitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a decimal u128 string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                value.parse().map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(U128Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn cache_toml_schema_round_trips() {
        let mut cache = Cache::default();
        cache.stamps.insert(
            "global".to_string(),
            ScopeStamp {
                last_pulled_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
                live_source_max_mtime_nanos: 1_700_000_000_123_456_789,
                mirror_content_hash: "abc123".to_string(),
            },
        );

        let serialized = toml::to_string_pretty(&cache).unwrap();
        assert_eq!(
            serialized,
            "[stamps.global]\nlive_source_max_mtime_nanos = \"1700000000123456789\"\nmirror_content_hash = \"abc123\"\n\n[stamps.global.last_pulled_at]\nsecs_since_epoch = 1700000000\nnanos_since_epoch = 0\n"
        );

        let parsed: Cache = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed, cache);
    }
}
