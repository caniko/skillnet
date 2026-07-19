//! Manifest-driven skill bundles.
//!
//! A manifest is an opt-in composition layer. It leaves the canonical skill
//! store unchanged, exposes only `entrypoint` skills in a view, and places
//! reference/dependency skills below the generated bundle's hidden
//! `.skillnet/deps` directory.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::{unix::fs as unix_fs, unix::fs::PermissionsExt},
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{manifest, mirror::mirror_skill_dirs};

#[derive(Debug, Clone)]
pub struct BundlePlan {
    pub bundle_root: Utf8PathBuf,
    skills: BTreeMap<String, SkillBundle>,
    expected: BTreeMap<String, Utf8PathBuf>,
}

#[derive(Debug, Clone)]
struct SkillBundle {
    source: Utf8PathBuf,
    source_is_file: bool,
    role: String,
    dependencies: Vec<String>,
}

pub fn plan(
    canonical: &Utf8Path,
    scope_name: &str,
    data_dir: &Utf8Path,
) -> Result<Option<BundlePlan>> {
    let Some(manifest) = manifest::load(canonical)? else {
        return Ok(None);
    };
    if !canonical.is_dir() {
        bail!("manifest canonical root does not exist or is not a directory: {canonical}");
    }
    validate_scope_name(scope_name)?;

    let sources: BTreeMap<String, Utf8PathBuf> = mirror_skill_dirs(canonical)?
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .context("canonical skill directory has no final component")?
                .to_string();
            Ok((name, path))
        })
        .collect::<Result<_>>()?;
    validate_dependencies("manifest defaults", &manifest.document.default_dependencies)?;

    let mut skills = BTreeMap::new();
    for (name, source) in &sources {
        let spec = manifest.document.skills.get(name);
        let role = spec
            .map(|spec| spec.role.clone())
            .unwrap_or_else(|| "entrypoint".to_string());
        let mut dependencies = manifest.document.default_dependencies.clone();
        if let Some(spec) = spec {
            dependencies.extend(spec.dependencies.iter().cloned());
        }
        validate_skill_name(name, "skill")?;
        validate_role(name, &role)?;
        validate_dependencies(name, &dependencies)?;
        skills.insert(
            name.clone(),
            SkillBundle {
                source: source.clone(),
                source_is_file: false,
                role,
                dependencies,
            },
        );
    }
    for (name, spec) in &manifest.document.skills {
        validate_skill_name(name, "manifest skill")?;
        if !sources.contains_key(name) {
            let Some(source) = &spec.source else {
                bail!(
                    "Skillnet manifest lists `{name}`, but no canonical skill directory exists and no source was provided"
                );
            };
            let source_path = confined_source_path(canonical, source)?;
            if !source_path.is_file() {
                bail!("Skillnet source for `{name}` is not a file: {source_path}");
            }
            skills.insert(
                name.clone(),
                SkillBundle {
                    source: source_path,
                    source_is_file: true,
                    role: spec.role.clone(),
                    dependencies: spec.dependencies.clone(),
                },
            );
        } else if let Some(source) = &spec.source {
            let source_path = confined_source_path(canonical, source)?;
            if !source_path.is_dir() {
                bail!("Skillnet source for `{name}` is not a directory: {source_path}");
            }
        }
        validate_role(name, &spec.role)?;
        validate_dependencies(name, &spec.dependencies)?;
    }
    for (name, spec) in &manifest.document.skills {
        for dependency in &spec.dependencies {
            if !skills.contains_key(dependency) {
                bail!("Skillnet skill `{name}` depends on missing skill `{dependency}`");
            }
            if dependency == name {
                bail!("Skillnet skill `{name}` cannot depend on itself");
            }
        }
    }
    detect_cycles(&skills)?;

    let bundle_root = data_dir.join("bundles").join(scope_name);
    let expected = skills
        .iter()
        .filter(|(_, skill)| skill.role == "entrypoint")
        .map(|(name, _)| (name.clone(), bundle_root.join(name)))
        .collect();
    Ok(Some(BundlePlan {
        bundle_root,
        skills,
        expected,
    }))
}

impl BundlePlan {
    pub fn expected_links(&self) -> &BTreeMap<String, Utf8PathBuf> {
        &self.expected
    }

    pub fn skill_names(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_str)
    }

    pub fn dependencies(&self, skill: &str) -> Option<&[String]> {
        self.skills
            .get(skill)
            .map(|skill| skill.dependencies.as_slice())
    }

    pub fn materialize(&self) -> Result<()> {
        fs::create_dir_all(&self.bundle_root)
            .with_context(|| format!("failed to create bundle root {}", self.bundle_root))?;
        for (name, skill) in &self.skills {
            let staging = self
                .bundle_root
                .join(format!(".{name}.skillnet-tmp-{}", std::process::id()));
            if staging.exists() {
                remove_entry(&staging)?;
            }
            fs::create_dir_all(&staging)
                .with_context(|| format!("failed to create bundle staging directory {staging}"))?;
            if skill.source_is_file {
                unix_fs::symlink(&skill.source, staging.join("SKILL.md"))
                    .with_context(|| format!("failed to link source file {}", skill.source))?;
            } else {
                link_tree_children(&skill.source, &staging)?;
            }
            let deps = staging.join(".skillnet/deps");
            if !skill.dependencies.is_empty() {
                fs::create_dir_all(&deps)
                    .with_context(|| format!("failed to create dependency directory {deps}"))?;
                for dependency in &skill.dependencies {
                    let target = self.bundle_root.join(dependency);
                    let link = deps.join(dependency);
                    unix_fs::symlink(&target, &link).with_context(|| {
                        format!("failed to link dependency {dependency} into {name}")
                    })?;
                }
            }
            let destination = self.bundle_root.join(name);
            if destination.exists() || fs::symlink_metadata(&destination).is_ok() {
                remove_entry(&destination)?;
            }
            fs::rename(&staging, &destination).with_context(|| {
                format!("failed to publish generated bundle skill {destination}")
            })?;
        }

        let expected_names: BTreeSet<&str> = self.skills.keys().map(String::as_str).collect();
        for entry in fs::read_dir(&self.bundle_root)
            .with_context(|| format!("failed to inspect bundle root {}", self.bundle_root))?
        {
            let entry = entry?;
            let path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow::anyhow!("non-UTF-8 bundle path: {}", path.display()))?;
            let Some(name) = path.file_name() else {
                continue;
            };
            if expected_names.contains(name) {
                continue;
            }
            if name.starts_with('.') && !name.contains(".skillnet-tmp-") {
                continue;
            }
            remove_entry(&path)
                .with_context(|| format!("failed to remove stale generated bundle entry {path}"))?;
        }
        Ok(())
    }
}

fn link_tree_children(source: &Utf8Path, staging: &Utf8Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read canonical skill {source}"))?
    {
        let entry = entry?;
        let child = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 skill path: {}", path.display()))?;
        let name = child
            .file_name()
            .context("canonical skill child has no final component")?;
        unix_fs::symlink(&child, staging.join(name))
            .with_context(|| format!("failed to link bundle child {child}"))?;
    }
    Ok(())
}

fn remove_entry(path: &Utf8Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect generated bundle entry {path}"))?;
    if metadata.file_type().is_dir() {
        make_tree_writable(path)?;
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove generated bundle directory {path}"))
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove generated bundle entry {path}"))
    }
}

fn make_tree_writable(path: &Utf8Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect generated bundle path {path}"))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o700);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to make generated bundle path writable {path}"))?;
    if metadata.file_type().is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow::anyhow!("non-UTF-8 generated path: {}", path.display()))?;
            make_tree_writable(&child)?;
        }
    }
    Ok(())
}

fn confined_source_path(canonical: &Utf8Path, source: &str) -> Result<Utf8PathBuf> {
    let path = canonical.join(source);
    let root = canonical
        .as_std_path()
        .canonicalize()
        .with_context(|| format!("failed to canonicalize Skillnet root {canonical}"))?;
    let resolved = path
        .as_std_path()
        .canonicalize()
        .with_context(|| format!("failed to resolve Skillnet source `{source}`"))?;
    if resolved != root && !resolved.starts_with(&root) {
        bail!("Skillnet source `{source}` escapes canonical root {canonical}");
    }
    Utf8PathBuf::from_path_buf(resolved)
        .map_err(|path| anyhow::anyhow!("non-UTF-8 Skillnet source path: {}", path.display()))
}

fn validate_scope_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("invalid Skillnet bundle scope name `{name}`");
    }
    Ok(())
}

fn validate_skill_name(name: &str, kind: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("invalid {kind} name `{name}`; names must be single path components");
    }
    Ok(())
}

fn validate_role(name: &str, role: &str) -> Result<()> {
    if matches!(role, "entrypoint" | "reference") {
        return Ok(());
    }
    bail!("Skillnet skill `{name}` has unsupported role `{role}`")
}

fn validate_dependencies(name: &str, dependencies: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for dependency in dependencies {
        validate_skill_name(dependency, "dependency")?;
        if !seen.insert(dependency) {
            bail!("Skillnet skill `{name}` lists duplicate dependency `{dependency}`");
        }
    }
    Ok(())
}

fn detect_cycles(skills: &BTreeMap<String, SkillBundle>) -> Result<()> {
    fn visit(
        name: &str,
        skills: &BTreeMap<String, SkillBundle>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            bail!("cycle detected in Skillnet dependencies at `{name}`");
        }
        let skill = skills
            .get(name)
            .with_context(|| format!("dependency graph references unknown skill `{name}`"))?;
        for dependency in &skill.dependencies {
            visit(dependency, skills, visiting, visited)?;
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for name in skills.keys() {
        visit(name, skills, &mut visiting, &mut visited)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    #[test]
    fn bundles_entrypoints_and_hides_references() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
        let data = Utf8PathBuf::from_path_buf(tmp.path().join("data")).unwrap();
        fs::create_dir_all(root.join("fix-loop")).unwrap();
        fs::create_dir_all(root.join("fix-loop-ref")).unwrap();
        fs::write(root.join("fix-loop/SKILL.md"), "entrypoint").unwrap();
        fs::write(root.join("fix-loop-ref/SKILL.md"), "reference").unwrap();
        fs::write(
            root.join(manifest::MANIFEST_FILE),
            r#"
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
}
schemaVersion = 1
skills: Mapping<String, Skill> = new {
  ["fix-loop"] = new { dependencies = List("fix-loop-ref") }
  ["fix-loop-ref"] = new { role = "reference" }
}
"#,
        )
        .unwrap();

        let plan = plan(&root, "global", &data).unwrap().unwrap();
        assert!(plan.expected_links().contains_key("fix-loop"));
        assert!(!plan.expected_links().contains_key("fix-loop-ref"));
        plan.materialize().unwrap();

        assert!(plan.bundle_root.join("fix-loop/SKILL.md").is_symlink());
        assert!(plan
            .bundle_root
            .join("fix-loop/.skillnet/deps/fix-loop-ref")
            .is_symlink());
        assert!(!plan.bundle_root.join(".fix-loop").exists());
    }

    #[test]
    fn missing_dependency_is_rejected_before_materialization() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
        let data = Utf8PathBuf::from_path_buf(tmp.path().join("data")).unwrap();
        fs::create_dir_all(root.join("entry")).unwrap();
        fs::write(root.join("entry/SKILL.md"), "entrypoint").unwrap();
        fs::write(
            root.join(manifest::MANIFEST_FILE),
            r#"
class Skill { dependencies: Listing<String> = new {} }
schemaVersion = 1
skills: Mapping<String, Skill> = new {
  ["entry"] = new { dependencies = List("missing") }
}
"#,
        )
        .unwrap();

        let error = plan(&root, "global", &data).unwrap_err().to_string();
        assert!(
            error.contains("depends on missing skill `missing`"),
            "{error}"
        );
        assert!(!data.exists());
    }

    #[test]
    fn source_file_can_supply_hidden_reference() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
        let data = Utf8PathBuf::from_path_buf(tmp.path().join("data")).unwrap();
        fs::create_dir_all(root.join("fix-loop/references")).unwrap();
        fs::write(root.join("fix-loop/SKILL.md"), "entrypoint").unwrap();
        fs::write(
            root.join("fix-loop/references/repair-contract.md"),
            "reference",
        )
        .unwrap();
        fs::write(
            root.join(manifest::MANIFEST_FILE),
            r#"
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
  source: String? = null
}
schemaVersion = 1
skills: Mapping<String, Skill> = new {
  ["fix-loop"] = new { dependencies = List("fix-loop-ref") }
  ["fix-loop-ref"] = new {
    role = "reference"
    source = "fix-loop/references/repair-contract.md"
  }
}
"#,
        )
        .unwrap();

        let plan = plan(&root, "global", &data).unwrap().unwrap();
        plan.materialize().unwrap();
        assert_eq!(
            fs::read_link(plan.bundle_root.join("fix-loop-ref/SKILL.md")).unwrap(),
            root.join("fix-loop/references/repair-contract.md")
                .as_std_path()
        );
    }
}
