use std::{env, fs};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{json, Map, Value};

const MANAGED_KEY: &str = "_skillnet_managed";

pub fn install(
    settings: &Utf8Path,
    events: &[String],
    matchers: &[String],
    dry_run: bool,
) -> Result<()> {
    let settings = expand_settings_path(settings)?;
    let events = normalized_values(events, "event")?;
    let matchers = normalized_values(matchers, "matcher")?;
    let mut document = load_settings(&settings)?;

    for event in &events {
        let entries = event_entries_mut(&mut document, event)?;
        entries.retain(|entry| !is_managed_entry(entry));
        for matcher in &matchers {
            entries.push(managed_entry(event, matcher));
        }
    }

    if dry_run {
        println!("would install skillnet hooks in {settings}");
        return Ok(());
    }

    write_settings(&settings, &document)?;
    println!("installed");
    Ok(())
}

pub fn uninstall(settings: &Utf8Path) -> Result<()> {
    let settings = expand_settings_path(settings)?;
    let existed = settings.exists();
    let mut document = load_settings(&settings)?;
    let original = document.clone();
    remove_managed_entries(&mut document)?;
    if !existed || document == original {
        println!("uninstalled");
        return Ok(());
    }
    write_settings(&settings, &document)?;
    println!("uninstalled");
    Ok(())
}

pub fn is_installed(settings: &Utf8Path) -> Result<bool> {
    let settings = expand_settings_path(settings)?;
    let document = load_settings(&settings)?;
    let Some(hooks) = document.get("hooks") else {
        return Ok(false);
    };
    let hooks = hooks
        .as_object()
        .with_context(|| format!("{settings}: top-level `hooks` must be a JSON object"))?;
    Ok(hooks.values().any(|entries| {
        entries
            .as_array()
            .is_some_and(|entries| entries.iter().any(is_managed_entry))
    }))
}

fn load_settings(path: &Utf8Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Claude settings file {path}"))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "failed to parse Claude settings JSON at {path}; remove comments or fix the JSON \
             before running `skillnet hook install`"
        )
    })?;
    if !value.is_object() {
        bail!("{path}: Claude settings root must be a JSON object");
    }
    Ok(value)
}

fn event_entries_mut<'a>(document: &'a mut Value, event: &str) -> Result<&'a mut Vec<Value>> {
    let root = document
        .as_object_mut()
        .context("Claude settings root must be a JSON object")?;
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("top-level `hooks` must be a JSON object")?;
    hooks
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .with_context(|| format!("hooks.{event} must be a JSON array"))
}

fn remove_managed_entries(document: &mut Value) -> Result<()> {
    let Some(root) = document.as_object_mut() else {
        bail!("Claude settings root must be a JSON object");
    };
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_object_mut()
        .context("top-level `hooks` must be a JSON object")?;
    for (event, entries) in hooks.iter_mut() {
        let entries = entries
            .as_array_mut()
            .with_context(|| format!("hooks.{event} must be a JSON array"))?;
        entries.retain(|entry| !is_managed_entry(entry));
    }
    hooks.retain(|_, entries| entries.as_array().is_none_or(|entries| !entries.is_empty()));
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}

fn managed_entry(event: &str, matcher: &str) -> Value {
    json!({
        "matcher": matcher,
        MANAGED_KEY: true,
        "hooks": [{
            "type": "command",
            "command": format!("skillnet hook ingest --event {event}"),
        }],
    })
}

fn is_managed_entry(entry: &Value) -> bool {
    entry
        .get(MANAGED_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn normalized_values(values: &[String], label: &str) -> Result<Vec<String>> {
    let values: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if values.is_empty() {
        bail!("at least one hook {label} is required");
    }
    Ok(values)
}

fn write_settings(path: &Utf8Path, document: &Value) -> Result<()> {
    let text = format!("{}\n", serde_json::to_string_pretty(document)?);
    if path.exists() {
        let current = fs::read_to_string(path)
            .with_context(|| format!("failed to read Claude settings file {path}"))?;
        if current == text {
            return Ok(());
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    create_backup_once(path)?;

    let tmp = Utf8PathBuf::from(format!("{path}.tmp"));
    fs::write(&tmp, text).with_context(|| format!("failed to write temporary file {tmp}"))?;
    if path.exists() {
        let permissions = fs::metadata(path)
            .with_context(|| format!("failed to inspect {path}"))?
            .permissions();
        fs::set_permissions(&tmp, permissions)
            .with_context(|| format!("failed to preserve permissions on {tmp}"))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {path} with {tmp}"))?;
    Ok(())
}

fn create_backup_once(path: &Utf8Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let backup = Utf8PathBuf::from(format!("{path}.bak"));
    if backup.exists() {
        return Ok(());
    }
    fs::copy(path, &backup).with_context(|| format!("failed to create backup {backup}"))?;
    Ok(())
}

fn expand_settings_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let raw = path.as_str();
    if raw == "$HOME" {
        return Ok(Utf8PathBuf::from(home_dir()?));
    }
    if let Some(rest) = raw.strip_prefix("$HOME/") {
        return Ok(Utf8PathBuf::from(home_dir()?).join(rest));
    }
    if raw == "~" {
        return Ok(Utf8PathBuf::from(home_dir()?));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(Utf8PathBuf::from(home_dir()?).join(rest));
    }
    Ok(path.to_path_buf())
}

fn home_dir() -> Result<String> {
    env::var("HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .context("HOME is not set; pass --settings explicitly")
}
