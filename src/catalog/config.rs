use std::fs;

use anyhow::{Context as AnyhowContext, Result};
use camino::Utf8Path;
use serde::Deserialize;

use super::entry::SkillEntry;

pub(super) const VALID_GLOBAL_CATEGORIES: &[&str] = &[
    "rust",
    "nix",
    "ci-release",
    "docs-sites",
    "planning",
    "routing",
    "agent-tools",
    "vendor-platforms",
    "creative-media",
];
pub(super) const VALID_PROJECT_CATEGORIES: &[&str] = &[
    "project-runtime",
    "test-fix-loop",
    "investigation",
    "deployment",
    "review",
    "internal-runtime",
    "domain-workflow",
];
pub(super) const VALID_SCOPES: &[&str] = &["global", "project", "plugin", "vendor"];
pub(super) const VALID_STATUSES: &[&str] =
    &["active", "reference", "internal", "experimental", "retired"];

#[derive(Debug, Default, Deserialize)]
pub(super) struct CatalogConfig {
    #[serde(default)]
    pub(super) settings: CatalogSettings,
    #[serde(default)]
    pub(super) rules: Vec<CatalogRule>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CatalogSettings {
    #[serde(default = "default_large_skill_line_threshold")]
    pub(super) large_skill_line_threshold: usize,
}

impl Default for CatalogSettings {
    fn default() -> Self {
        Self {
            large_skill_line_threshold: default_large_skill_line_threshold(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CatalogRule {
    path_prefix: Option<String>,
    name: Option<String>,
    name_prefix: Option<String>,
    name_suffix: Option<String>,
    project: Option<String>,
    category: Option<String>,
    scope: Option<String>,
    status: Option<String>,
    collision_note: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    related_skills: Vec<String>,
}

impl CatalogConfig {
    pub(super) fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read catalog config file {path}"))?;
        toml::from_str(&text).with_context(|| format!("failed to parse catalog config file {path}"))
    }
}

impl CatalogRule {
    pub(super) fn matches(&self, entry: &SkillEntry, rel: &str) -> bool {
        self.path_prefix
            .as_deref()
            .is_none_or(|prefix| rel.starts_with(prefix))
            && self.name.as_deref().is_none_or(|name| entry.name == name)
            && self
                .name_prefix
                .as_deref()
                .is_none_or(|prefix| entry.name.starts_with(prefix))
            && self
                .name_suffix
                .as_deref()
                .is_none_or(|suffix| entry.name.ends_with(suffix))
            && self
                .project
                .as_deref()
                .is_none_or(|project| entry.project.as_deref() == Some(project))
    }

    pub(super) fn apply(&self, entry: &mut SkillEntry) {
        if let Some(category) = &self.category {
            entry.category = Some(category.clone());
        }
        if let Some(scope) = &self.scope {
            entry.scope = scope.clone();
        }
        if let Some(status) = &self.status {
            entry.status = status.clone();
        }
        if let Some(collision_note) = &self.collision_note {
            entry.collision_note = Some(collision_note.clone());
        }
        entry.tags.extend(self.tags.clone());
        entry.related_skills.extend(self.related_skills.clone());
    }
}

fn default_large_skill_line_threshold() -> usize {
    220
}
