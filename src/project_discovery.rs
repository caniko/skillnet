use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDiscoveryConfig {
    pub project_tree: String,
    #[serde(default = "default_classes")]
    pub classes: Vec<String>,
    #[serde(default = "default_marker")]
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProject {
    pub name: String,
    pub path: Utf8PathBuf,
    pub relative_path: Utf8PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectTreeDocument {
    schema_version: u32,
    root: Utf8PathBuf,
    layout: ProjectTreeLayout,
}

#[derive(Debug, Deserialize)]
struct ProjectTreeLayout {
    primary: BTreeMap<String, String>,
}

fn default_classes() -> Vec<String> {
    vec!["owned".into(), "forks".into()]
}

fn default_marker() -> String {
    ".skills".into()
}

pub fn discover(config: &ProjectDiscoveryConfig) -> Result<Vec<DiscoveredProject>> {
    if config.classes.is_empty() {
        bail!("project_discovery.classes must contain at least one class");
    }

    let project_tree_path = crate::config::expand_path(&config.project_tree)
        .with_context(|| format!("resolve project tree config `{}`", config.project_tree))?;
    let text = fs::read_to_string(project_tree_path.as_std_path())
        .with_context(|| format!("read project tree config `{}`", project_tree_path))?;
    let tree: ProjectTreeDocument = serde_json::from_str(&text)
        .with_context(|| format!("parse project tree config `{}`", project_tree_path))?;
    if tree.schema_version != 1 {
        bail!(
            "unsupported project tree schema version {} in `{}` (expected 1)",
            tree.schema_version,
            project_tree_path
        );
    }
    if !tree.root.is_absolute() {
        bail!("project tree root `{}` must be absolute", tree.root);
    }
    if !tree.root.is_dir() {
        bail!("project tree root `{}` is not a directory", tree.root);
    }
    let marker = relative_path(&config.marker, "project_discovery.marker")?;

    let mut projects = Vec::new();
    let mut names = BTreeSet::new();
    let mut classes = BTreeSet::new();
    for class in &config.classes {
        if !classes.insert(class) {
            bail!("project_discovery.classes contains duplicate class `{class}`");
        }
        let class_rel = tree
            .layout
            .primary
            .get(class)
            .with_context(|| format!("project tree has no primary class `{class}`"))?;
        let class_rel = relative_path(class_rel, &format!("project tree class `{class}`"))?;
        let class_rel = Utf8PathBuf::from(class_rel.to_string_lossy().as_ref());
        let class_root = tree.root.join(class_rel);
        if !class_root.is_dir() {
            bail!("project tree class `{class}` root `{class_root}` is not a directory");
        }

        let mut entries = fs::read_dir(class_root.as_std_path())
            .with_context(|| format!("read project class `{class}` at `{class_root}`"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("read project class `{class}` at `{class_root}`"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("inspect project candidate `{}`", path.display()))?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            if !path.join(".git").exists() {
                continue;
            }

            let marker_path = path.join(&marker);
            let marker_metadata = match fs::symlink_metadata(&marker_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("inspect Skillnet marker `{}`", marker_path.display())
                    })
                }
            };
            if !marker_metadata.file_type().is_dir() || marker_metadata.file_type().is_symlink() {
                continue;
            }

            let path = Utf8PathBuf::from_path_buf(path)
                .map_err(|path| anyhow::anyhow!("project path is not UTF-8: {}", path.display()))?;
            let relative_path = path
                .as_std_path()
                .strip_prefix(tree.root.as_std_path())
                .with_context(|| format!("project `{path}` is outside `{}`", tree.root))?;
            let relative_path = Utf8PathBuf::from_path_buf(relative_path.to_path_buf())
                .map_err(|path| anyhow::anyhow!("project path is not UTF-8: {}", path.display()))?;
            let name = path
                .file_name()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .with_context(|| format!("project path `{path}` has no repository name"))?;
            if !names.insert(name.clone()) {
                bail!(
                    "discovered project name `{name}` is used more than once; add an explicit project entry to disambiguate"
                );
            }
            projects.push(DiscoveredProject {
                name,
                path,
                relative_path,
            });
        }
    }

    projects.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(projects)
}

fn relative_path(raw: &str, label: &str) -> Result<std::path::PathBuf> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("{label} must be a non-empty relative path without `..`: `{raw}`");
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;

    fn tree_config(root: &Path) -> ProjectDiscoveryConfig {
        let project_tree = root.join("project-tree.json");
        fs::write(
            &project_tree,
            serde_json::json!({
                "schemaVersion": 1,
                "root": root,
                "layout": {
                    "primary": {"owned": "owned", "forks": "forks", "upstream": "upstream"}
                }
            })
            .to_string(),
        )
        .unwrap();
        ProjectDiscoveryConfig {
            project_tree: project_tree.display().to_string(),
            classes: vec!["owned".into(), "forks".into()],
            marker: ".skills".into(),
        }
    }

    fn checkout(root: &Path, rel: &str, marker: bool) {
        let path = root.join(rel);
        fs::create_dir_all(path.join(".git")).unwrap();
        if marker {
            fs::create_dir_all(path.join(".skills")).unwrap();
        }
    }

    #[test]
    fn discovers_marked_direct_checkouts_and_excludes_other_classes() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("owned")).unwrap();
        fs::create_dir_all(temp.path().join("forks")).unwrap();
        fs::create_dir_all(temp.path().join("personal")).unwrap();
        fs::create_dir_all(temp.path().join("worktrees")).unwrap();
        fs::create_dir_all(temp.path().join("archives")).unwrap();
        checkout(temp.path(), "owned/alpha", true);
        checkout(temp.path(), "owned/markerless", false);
        fs::create_dir_all(temp.path().join("owned/not-git/.skills")).unwrap();
        checkout(temp.path(), "owned/nested/ignored", true);
        checkout(temp.path(), "forks/beta", true);
        checkout(temp.path(), "personal/private", true);
        checkout(temp.path(), "worktrees/generated", true);
        checkout(temp.path(), "archives/old", true);

        let projects = discover(&tree_config(temp.path())).unwrap();
        assert_eq!(
            projects
                .iter()
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(projects[0].relative_path, Utf8PathBuf::from("owned/alpha"));
    }

    #[test]
    fn rejects_duplicate_leaf_names() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("owned")).unwrap();
        fs::create_dir_all(temp.path().join("forks")).unwrap();
        checkout(temp.path(), "owned/demo", true);
        checkout(temp.path(), "forks/demo", true);

        let error = discover(&tree_config(temp.path())).unwrap_err();
        assert!(error.to_string().contains("discovered project name `demo`"));
    }

    #[test]
    fn rejects_missing_or_malformed_project_tree() {
        let temp = tempdir().unwrap();
        let mut config = tree_config(temp.path());
        fs::remove_file(&config.project_tree).unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error.to_string().contains("read project tree config"));

        fs::write(&config.project_tree, "not json").unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error.to_string().contains("parse project tree config"));

        fs::write(
            &config.project_tree,
            serde_json::json!({
                "schemaVersion": 2,
                "root": temp.path(),
                "layout": {"primary": {"owned": "owned", "forks": "forks"}}
            })
            .to_string(),
        )
        .unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported project tree schema version 2"));

        fs::write(
            &config.project_tree,
            serde_json::json!({
                "schemaVersion": 1,
                "root": temp.path().join("missing-root"),
                "layout": {"primary": {"owned": "owned", "forks": "forks"}}
            })
            .to_string(),
        )
        .unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error.to_string().contains("project tree root"));

        fs::write(
            &config.project_tree,
            serde_json::json!({
                "schemaVersion": 1,
                "root": temp.path(),
                "layout": {"primary": {"owned": "owned", "forks": "forks"}}
            })
            .to_string(),
        )
        .unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error
            .to_string()
            .contains("project tree class `owned` root"));

        config.marker = "../skills".into();
        fs::write(
            &config.project_tree,
            serde_json::json!({
                "schemaVersion": 1,
                "root": temp.path(),
                "layout": {"primary": {"owned": "owned", "forks": "forks"}}
            })
            .to_string(),
        )
        .unwrap();
        let error = discover(&config).unwrap_err();
        assert!(error.to_string().contains("project_discovery.marker"));
    }

    #[cfg(unix)]
    #[test]
    fn excludes_symlinked_skill_markers() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("owned")).unwrap();
        fs::create_dir_all(temp.path().join("forks")).unwrap();
        checkout(temp.path(), "owned/symlinked", false);
        fs::create_dir_all(temp.path().join("real-skills")).unwrap();
        unix_fs::symlink(
            temp.path().join("real-skills"),
            temp.path().join("owned/symlinked/.skills"),
        )
        .unwrap();

        assert!(discover(&tree_config(temp.path())).unwrap().is_empty());
    }
}
