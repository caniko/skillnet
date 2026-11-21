//! Read model and option metadata from local T3 provider snapshots.

use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    #[serde(skip_deserializing)]
    pub instance_id: String,
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    pub status: String,
    pub models: Vec<Model>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_provider: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub option_descriptors: Vec<OptionDescriptor>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionDescriptor {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: Vec<ModelOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_value: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
}

/// Load enabled snapshots, ordered by provider instance ID (the file stem).
pub fn load_t3_caches(dir: &Utf8Path) -> Result<Vec<Provider>> {
    let mut providers = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read T3 caches {dir}"))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let snapshot = (|| -> Result<Provider> {
            let mut provider: Provider = serde_json::from_slice(&fs::read(&path)?)?;
            provider.instance_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .context("non-UTF-8 provider instance ID")?
                .to_owned();
            Ok(provider)
        })();
        match snapshot {
            Ok(provider) if provider.enabled => providers.push(provider),
            Ok(_) => {}
            Err(error) => eprintln!("warning: ignoring T3 snapshot {}: {error}", path.display()),
        }
    }
    providers.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    Ok(providers)
}
