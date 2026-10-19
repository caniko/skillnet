//! Pkl-backed Skillnet manifest loading.
//!
//! Manifests are evaluated with a deliberately restricted capability provider:
//! local imports may only read files below the canonical skill root and all
//! environment, network, temporary-directory, and glob access is rejected.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use pklr::capabilities::EvalCapabilities;
use serde::Deserialize;

pub const MANIFEST_FILE: &str = "Skillnet.pkl";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestDocument {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u64,
    #[serde(rename = "defaultDependencies", default)]
    pub default_dependencies: Vec<String>,
    #[serde(rename = "defaultUsers", default)]
    pub default_users: Option<Vec<String>>,
    #[serde(rename = "defaultHosts", default)]
    pub default_hosts: Option<Vec<String>>,
    #[serde(default)]
    pub skills: BTreeMap<String, SkillSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSpec {
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub users: Option<Vec<String>>,
    #[serde(default)]
    pub hosts: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub document: ManifestDocument,
    pub path: camino::Utf8PathBuf,
}

pub fn load(canonical: &Utf8Path) -> Result<Option<Manifest>> {
    load_path(&canonical.join(MANIFEST_FILE))
}

/// Load a manifest from an explicit immutable bundle path. Evaluation is
/// confined to the directory containing that manifest, so Pkl imports cannot
/// reach the user's writable canonical mirror or the network.
pub fn load_path(path: &Utf8Path) -> Result<Option<Manifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read Skillnet manifest {path}"))?;
    let root = path
        .parent()
        .context("Skillnet manifest has no parent")?
        .as_std_path()
        .canonicalize()
        .with_context(|| format!("failed to canonicalize manifest root {}", path))?;
    let manifest_path = path.as_std_path();
    let mut evaluator = pklr::Evaluator::with_capabilities(LocalCapabilities::new(root.clone()));
    evaluator.set_base_path(&root);
    let value = evaluator
        .eval_source(&source, manifest_path)
        .map_err(|error| anyhow::anyhow!("failed to evaluate {path}: {error}"))?;
    let document: ManifestDocument = serde_json::from_value(value.to_json())
        .with_context(|| format!("invalid evaluated Skillnet manifest {path}"))?;
    if !matches!(document.schema_version, 1..=3) {
        bail!(
            "unsupported Skillnet manifest schemaVersion {} in {path}; expected 1, 2 or 3",
            document.schema_version
        );
    }
    if document.schema_version == 1
        && (document.default_users.is_some()
            || document.skills.values().any(|skill| skill.users.is_some()))
    {
        bail!(
            "Skillnet manifest {path} uses user access fields but declares schemaVersion 1; use schemaVersion 2"
        );
    }
    validate_user_list(document.default_users.as_deref(), "defaultUsers", path)?;
    if document.schema_version < 3
        && (document.default_hosts.is_some()
            || document.skills.values().any(|skill| skill.hosts.is_some()))
    {
        bail!("Skillnet manifest {path} uses host access fields; use schemaVersion 3");
    }
    validate_host_list(document.default_hosts.as_deref(), "defaultHosts", path)?;
    for (name, skill) in &document.skills {
        validate_host_list(
            skill.hosts.as_deref(),
            &format!("skill `{name}` hosts"),
            path,
        )?;
        validate_user_list(
            skill.users.as_deref(),
            &format!("skill `{name}` users"),
            path,
        )?;
    }
    Ok(Some(Manifest {
        document,
        path: path.to_path_buf(),
    }))
}

fn validate_user_list(users: Option<&[String]>, field: &str, path: &Utf8Path) -> Result<()> {
    let Some(users) = users else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    for user in users {
        if user.trim().is_empty() {
            bail!("Skillnet manifest {path} has an empty username in {field}");
        }
        if !seen.insert(user) {
            bail!("Skillnet manifest {path} lists duplicate username `{user}` in {field}");
        }
    }
    Ok(())
}

fn default_role() -> String {
    "entrypoint".to_string()
}

pub(crate) fn validate_host(host: &str) -> Result<()> {
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
    {
        bail!("invalid Skillnet host `{host}`; use a nonempty explicit host name");
    }
    Ok(())
}

fn validate_host_list(hosts: Option<&[String]>, field: &str, path: &Utf8Path) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for host in hosts.unwrap_or_default() {
        validate_host(host).with_context(|| format!("Skillnet manifest {path}: {field}"))?;
        if !seen.insert(host) {
            bail!("Skillnet manifest {path} lists duplicate host `{host}` in {field}");
        }
    }
    Ok(())
}

struct LocalCapabilities {
    root: PathBuf,
}

impl LocalCapabilities {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn confined(&self, path: &Path) -> pklr::Result<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|error| pklr::Error::Io(candidate.clone(), error))?;
        if canonical == self.root || canonical.starts_with(&self.root) {
            Ok(canonical)
        } else {
            Err(pklr::Error::Unsupported(format!(
                "Pkl path escapes Skillnet manifest root: {}",
                path.display()
            )))
        }
    }
}

impl EvalCapabilities for LocalCapabilities {
    fn read_to_string(&mut self, path: &Path) -> pklr::Result<String> {
        let path = self.confined(path)?;
        fs::read_to_string(&path).map_err(|error| pklr::Error::Io(path, error))
    }

    fn path_exists(&mut self, path: &Path) -> pklr::Result<bool> {
        Ok(self.confined(path).is_ok())
    }

    fn canonicalize(&mut self, path: &Path) -> pklr::Result<PathBuf> {
        self.confined(path)
    }

    fn read_bytes(&mut self, path: &Path) -> pklr::Result<Vec<u8>> {
        let path = self.confined(path)?;
        fs::read(&path).map_err(|error| pklr::Error::Io(path, error))
    }

    fn create_dir_all(&mut self, path: &Path) -> pklr::Result<()> {
        Err(pklr::Error::Unsupported(format!(
            "filesystem mutations are disabled while evaluating Skillnet.pkl: {}",
            path.display()
        )))
    }

    fn write_atomic(&mut self, path: &Path, _bytes: &[u8]) -> pklr::Result<()> {
        Err(pklr::Error::Unsupported(format!(
            "filesystem mutations are disabled while evaluating Skillnet.pkl: {}",
            path.display()
        )))
    }

    fn remove_file(&mut self, path: &Path) -> pklr::Result<()> {
        Err(pklr::Error::Unsupported(format!(
            "filesystem mutations are disabled while evaluating Skillnet.pkl: {}",
            path.display()
        )))
    }

    fn read_env(&mut self, name: &str) -> pklr::Result<Option<String>> {
        Err(pklr::Error::Unsupported(format!(
            "environment access is disabled while evaluating Skillnet.pkl: {name}"
        )))
    }

    fn env_vars(&mut self) -> pklr::Result<Vec<(String, String)>> {
        Err(pklr::Error::Unsupported(
            "environment access is disabled while evaluating Skillnet.pkl".to_string(),
        ))
    }

    fn fetch_text(&mut self, url: &str) -> pklr::Result<String> {
        Err(pklr::Error::Unsupported(format!(
            "network access is disabled while evaluating Skillnet.pkl: {url}"
        )))
    }

    fn fetch_bytes(&mut self, url: &str) -> pklr::Result<Vec<u8>> {
        Err(pklr::Error::Unsupported(format!(
            "network access is disabled while evaluating Skillnet.pkl: {url}"
        )))
    }

    fn temp_dir(&mut self, prefix: &str) -> pklr::Result<PathBuf> {
        Err(pklr::Error::Unsupported(format!(
            "temporary directories are disabled while evaluating Skillnet.pkl: {prefix}"
        )))
    }

    fn glob(&mut self, _base: &Path, pattern: &str) -> pklr::Result<Vec<PathBuf>> {
        Err(pklr::Error::Unsupported(format!(
            "glob access is disabled while evaluating Skillnet.pkl: {pattern}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    #[test]
    fn evaluates_typed_manifest() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
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

        let manifest = load(&root).unwrap().unwrap();
        assert_eq!(manifest.document.schema_version, 1);
        assert_eq!(manifest.document.default_users, None);
        assert!(manifest.document.default_dependencies.is_empty());
        assert_eq!(
            manifest.document.skills["fix-loop"].dependencies,
            vec!["fix-loop-ref"]
        );
        assert_eq!(manifest.document.skills["fix-loop-ref"].role, "reference");
    }

    #[test]
    fn evaluates_schema_v2_user_access() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            r#"
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
  users: Listing<String>? = null
}

schemaVersion = 2
defaultUsers = List("can", "dejana")
skills: Mapping<String, Skill> = new {
  ["shared"] = new {}
  ["admin"] = new { users = List("can") }
  ["hidden"] = new { users = List() }
}
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();
        assert_eq!(manifest.document.schema_version, 2);
        assert_eq!(
            manifest.document.default_users,
            Some(vec!["can".to_string(), "dejana".to_string()])
        );
        assert_eq!(
            manifest.document.skills["admin"].users,
            Some(vec!["can".to_string()])
        );
        assert_eq!(manifest.document.skills["hidden"].users, Some(Vec::new()));
    }

    #[test]
    fn rejects_access_fields_in_schema_v1() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            r#"
class Skill {
  users: Listing<String>? = null
}
schemaVersion = 1
skills: Mapping<String, Skill> = new {
  ["admin"] = new { users = List("can") }
}
"#,
        )
        .unwrap();

        let error = load(&root).unwrap_err().to_string();
        assert!(error.contains("schemaVersion 2"), "{error}");
    }

    #[test]
    fn rejects_environment_access() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        // The evaluator's capability boundary is exercised by an actual Pkl
        // resource read rather than by allowing a native fallback.
        fs::write(
            root.join(MANIFEST_FILE),
            r#"schemaVersion = 1
value = read("env:HOME")
"#,
        )
        .unwrap();

        let error = load(&root).unwrap_err().to_string();
        assert!(error.contains("environment access is disabled"), "{error}");
    }

    #[test]
    fn confines_binary_reads_and_rejects_filesystem_mutations() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("manifest");
        fs::create_dir(&root).unwrap();
        let outside = tmp.path().join("outside.bin");
        fs::write(&outside, [0, 255]).unwrap();
        let mut capabilities = LocalCapabilities::new(root.canonicalize().unwrap());

        let inside = root.join("inside.bin");
        fs::write(&inside, [1, 254]).unwrap();
        assert_eq!(capabilities.read_bytes(&inside).unwrap(), [1, 254]);
        assert!(capabilities.read_bytes(&outside).is_err());
        assert!(capabilities.env_vars().is_err());
        assert!(capabilities.create_dir_all(&root.join("created")).is_err());
        assert!(capabilities.write_atomic(&outside, b"replaced").is_err());
        assert!(capabilities.remove_file(&outside).is_err());
        assert_eq!(fs::read(&outside).unwrap(), [0, 255]);
        assert!(!root.join("created").exists());
    }
}
