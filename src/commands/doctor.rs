use std::{collections::BTreeSet, fmt, fs};

use anyhow::Result;
use camino::Utf8Path;

use super::Context;

const KNOWN_AGENTS: &[KnownAgent] = &[
    KnownAgent {
        label: "agents",
        sync_suffix: ".agents/skills",
    },
    KnownAgent {
        label: "claude",
        sync_suffix: ".claude/skills",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct KnownAgent {
    label: &'static str,
    sync_suffix: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub scope: String,
    pub kind: FindingKind,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    AsymmetricFanout,
    MissingSyncPath,
    SingletonGlobalScope,
}

impl FindingKind {
    fn label(&self) -> &'static str {
        match self {
            Self::AsymmetricFanout => "asymmetric fan-out",
            Self::MissingSyncPath => "missing sync path",
            Self::SingletonGlobalScope => "singleton global scope",
        }
    }
}

impl fmt::Display for FindingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

pub fn run(ctx: &Context) -> Result<Vec<Finding>> {
    let findings = lint(ctx)?;
    for finding in &findings {
        println!(
            "warn  {}  {}: {}",
            finding.scope, finding.kind, finding.detail
        );
    }
    Ok(findings)
}

pub fn lint(ctx: &Context) -> Result<Vec<Finding>> {
    let target = ctx.config.global_target(&ctx.mirror_root)?;
    let source_agents = source_agent_labels(&target.sources);
    let mut findings = Vec::new();

    if !source_agents.is_empty() && target.sync_paths.is_empty() {
        findings.push(Finding {
            scope: target.name.clone(),
            kind: FindingKind::SingletonGlobalScope,
            detail: format!(
                "sources include {} but sync_paths is empty",
                quoted_labels(&source_agents)
            ),
        });
    } else if source_agents.len() > 1 && target.sync_paths.len() < 2 {
        findings.push(Finding {
            scope: target.name.clone(),
            kind: FindingKind::SingletonGlobalScope,
            detail: format!(
                "sources include multiple canonical agents ({}) but sync_paths has {} target",
                quoted_labels(&source_agents),
                target.sync_paths.len()
            ),
        });
    }

    if source_agents.len() > 1 {
        for agent in KNOWN_AGENTS
            .iter()
            .filter(|agent| source_agents.contains(agent.label))
        {
            if !target
                .sync_paths
                .iter()
                .any(|path| has_sync_suffix(path, agent.sync_suffix))
            {
                findings.push(Finding {
                    scope: target.name.clone(),
                    kind: FindingKind::AsymmetricFanout,
                    detail: format!(
                        "sources include \"{}\" but sync_paths lacks \"{}\"",
                        agent.label, agent.sync_suffix
                    ),
                });
            }
        }
    }

    for path in &target.sync_paths {
        if let Some(detail) = invalid_sync_path_detail(path) {
            findings.push(Finding {
                scope: target.name.clone(),
                kind: FindingKind::MissingSyncPath,
                detail,
            });
        }
    }

    findings.sort_by(|left, right| {
        left.scope
            .cmp(&right.scope)
            .then(left.kind.cmp(&right.kind))
            .then(left.detail.cmp(&right.detail))
    });
    Ok(findings)
}

fn source_agent_labels(sources: &[crate::model::Source]) -> BTreeSet<&'static str> {
    sources
        .iter()
        .filter_map(|source| {
            KNOWN_AGENTS
                .iter()
                .find(|agent| agent.label == source.label)
                .map(|agent| agent.label)
        })
        .collect()
}

fn has_sync_suffix(path: &Utf8Path, suffix: &str) -> bool {
    path.as_str() == suffix || path.as_str().ends_with(&format!("/{suffix}"))
}

fn invalid_sync_path_detail(path: &Utf8Path) -> Option<String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => None,
        Ok(_) => Some(format!("{path} (exists but is not a directory)")),
        Err(_) => {
            let parent = path.parent()?;
            if parent.is_dir() {
                None
            } else {
                Some(format!("{path} (parent does not exist)"))
            }
        }
    }
}

fn quoted_labels(labels: &BTreeSet<&str>) -> String {
    labels
        .iter()
        .map(|label| format!("\"{label}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        commands::Context,
        config::{Config, DatabaseConfig, GlobalConfig, SourceConfig, SyncConfig},
    };
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    fn context_with_global(sources: Vec<SourceConfig>, sync_paths: Vec<Utf8PathBuf>) -> Context {
        let tmp = tempdir().unwrap();
        let mirror_root = Utf8PathBuf::from_path_buf(tmp.keep()).unwrap();
        Context {
            config_path: mirror_root.join("skillnet.toml"),
            catalog_config_path: mirror_root.join("skillnet.catalog.toml"),
            config: Config {
                global: GlobalConfig {
                    sources,
                    sync_paths: sync_paths
                        .into_iter()
                        .map(|path| path.to_string())
                        .collect(),
                    stale_codex_skill_paths: Vec::new(),
                },
                skills_root: None,
                mirror_root: None,
                sync: SyncConfig::default(),
                database: DatabaseConfig::default(),
                project_source_rules: Vec::new(),
                projects: Vec::new(),
            },
            mirror_root,
            dry_run: false,
            allow_dirty_destination: false,
        }
    }

    fn source(label: &str, path: &Utf8Path) -> SourceConfig {
        SourceConfig {
            label: label.to_string(),
            path: path.to_string(),
            priority: 1,
        }
    }

    #[test]
    fn lint_allows_single_agent_single_destination() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let agents = root.join(".agents/skills");
        fs::create_dir_all(&agents).unwrap();
        let ctx = context_with_global(vec![source("agents", &agents)], vec![agents]);

        let findings = lint(&ctx).unwrap();

        assert!(findings.is_empty());
    }

    #[test]
    fn lint_reports_asymmetric_fanout() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let agents = root.join(".agents/skills");
        let claude = root.join(".claude/skills");
        fs::create_dir_all(&agents).unwrap();
        fs::create_dir_all(&claude).unwrap();
        let ctx = context_with_global(
            vec![source("agents", &agents), source("claude", &claude)],
            vec![claude],
        );

        let findings = lint(&ctx).unwrap();

        assert!(findings
            .iter()
            .any(|finding| finding.kind == FindingKind::AsymmetricFanout
                && finding.detail.contains(".agents/skills")));
    }

    #[test]
    fn lint_reports_missing_sync_path_parent() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let agents = root.join(".agents/skills");
        let missing = root.join("missing/skills");
        fs::create_dir_all(&agents).unwrap();
        let ctx = context_with_global(vec![source("agents", &agents)], vec![agents, missing]);

        let findings = lint(&ctx).unwrap();

        assert!(findings
            .iter()
            .any(|finding| finding.kind == FindingKind::MissingSyncPath));
    }
}
