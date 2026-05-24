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
    let mut findings = Vec::new();
    for view in &target.views {
        if let Some(detail) = invalid_sync_path_detail(&view.path) {
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
        config::{Config, DatabaseConfig, GlobalConfig, SyncConfig, ViewConfig, ViewScope},
    };
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    fn context_with_global(views: Vec<Utf8PathBuf>) -> Context {
        let tmp = tempdir().unwrap();
        let mirror_root = Utf8PathBuf::from_path_buf(tmp.keep()).unwrap();
        Context {
            config_path: mirror_root.join("skillnet.toml"),
            catalog_config_path: mirror_root.join("skillnet.catalog.toml"),
            config: Config {
                global: GlobalConfig {
                    canonical_path: None,
                    views: views
                        .into_iter()
                        .map(|path| ViewConfig {
                            label: path.file_name().unwrap_or("view").to_string(),
                            path: path.to_string(),
                            scope: ViewScope::Global,
                        })
                        .collect(),
                },
                skills_root: None,
                mirror_root: None,
                sync: SyncConfig::default(),
                database: DatabaseConfig::default(),
                projects: Vec::new(),
            },
            mirror_root,
            dry_run: false,
            allow_dirty_destination: false,
        }
    }

    #[test]
    fn lint_allows_single_agent_single_destination() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let agents = root.join(".agents/skills");
        fs::create_dir_all(&agents).unwrap();
        let ctx = context_with_global(vec![agents]);

        let findings = lint(&ctx).unwrap();

        assert!(findings.is_empty());
    }

    #[test]
    #[ignore = "P9 rewrites doctor invariants for Option B views"]
    fn lint_reports_asymmetric_fanout() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let agents = root.join(".agents/skills");
        let claude = root.join(".claude/skills");
        fs::create_dir_all(&agents).unwrap();
        fs::create_dir_all(&claude).unwrap();
        let ctx = context_with_global(vec![claude]);

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
        let ctx = context_with_global(vec![agents, missing]);

        let findings = lint(&ctx).unwrap();

        assert!(findings
            .iter()
            .any(|finding| finding.kind == FindingKind::MissingSyncPath));
    }
}
