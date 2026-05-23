use std::{env, fs};

use anyhow::{anyhow, bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::model::{Source, Target};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    #[serde(default)]
    pub project_source_rules: Vec<ProjectSourceRule>,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Deserialize)]
pub struct GlobalConfig {
    pub sources: Vec<SourceConfig>,
    pub sync_paths: Vec<String>,
    pub stale_codex_skill_paths: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SourceConfig {
    pub label: String,
    pub path: String,
    pub priority: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProjectSourceRule {
    pub label: String,
    pub rel: String,
    pub priority: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub extra_sources: Vec<ProjectSourceRule>,
}

impl Config {
    pub fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {path}"))?;
        toml::from_str(&text).with_context(|| format!("failed to parse config file {path}"))
    }

    pub fn targets(&self, mirror_root: &Utf8Path) -> Result<Vec<Target>> {
        let mut targets = Vec::with_capacity(self.projects.len() + 1);
        targets.push(self.global_target(mirror_root)?);
        for project in &self.projects {
            targets.push(self.project_target(mirror_root, project)?);
        }
        Ok(targets)
    }

    pub fn global_target(&self, mirror_root: &Utf8Path) -> Result<Target> {
        Ok(Target {
            name: "global".to_string(),
            mirror_path: mirror_root.join("global"),
            sources: self
                .global
                .sources
                .iter()
                .map(|s| {
                    Ok(Source {
                        label: s.label.clone(),
                        path: expand_path(&s.path)?,
                        priority: s.priority,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            sync_paths: expand_paths(&self.global.sync_paths)?,
            stale_codex_skill_paths: expand_paths(&self.global.stale_codex_skill_paths)?,
        })
    }

    pub fn project_target(
        &self,
        mirror_root: &Utf8Path,
        project: &ProjectConfig,
    ) -> Result<Target> {
        let project_root = expand_path(&project.path)?;
        let mut rules = self.project_source_rules.clone();
        rules.extend(project.extra_sources.clone());

        Ok(Target {
            name: project.name.clone(),
            mirror_path: mirror_root.join("projects").join(&project.name),
            sources: rules
                .into_iter()
                .map(|rule| Source {
                    label: rule.label,
                    path: project_root.join(rule.rel),
                    priority: rule.priority,
                })
                .collect(),
            sync_paths: vec![
                project_root.join(".agents/skills"),
                project_root.join(".claude/skills"),
            ],
            stale_codex_skill_paths: vec![project_root.join(".codex/skills")],
        })
    }
}

pub fn expand_path(raw: &str) -> Result<Utf8PathBuf> {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    let path = Utf8PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        env::current_dir()
            .map_err(anyhow::Error::from)
            .and_then(|p| Utf8PathBuf::from_path_buf(p).map_err(|_| anyhow!("cwd is not UTF-8")))
            .map(|cwd| cwd.join(path))
    }
}

fn expand_paths(raws: &[String]) -> Result<Vec<Utf8PathBuf>> {
    raws.iter().map(|raw| expand_path(raw)).collect()
}

fn home_dir() -> Result<Utf8PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    if home.is_empty() {
        bail!("HOME is empty");
    }
    Ok(Utf8PathBuf::from(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_prefix() {
        let home = env::var("HOME").unwrap();
        let path = expand_path("~/x/y").unwrap();
        assert_eq!(path, Utf8PathBuf::from(home).join("x/y"));
    }
}
