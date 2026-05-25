use std::{fmt, fs};

use anyhow::Result;
use camino::Utf8Path;

use super::Context;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub scope: String,
    pub kind: FindingKind,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    MissingSyncPath,
}

impl FindingKind {
    fn label(&self) -> &'static str {
        match self {
            Self::MissingSyncPath => "missing sync path",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        commands::Context,
        config::{Config, DatabaseConfig, GlobalConfig, ViewConfig, ViewScope},
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
