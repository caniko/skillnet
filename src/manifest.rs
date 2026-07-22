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

use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use pklr::capabilities::{BoxFuture, EvalCapabilities};
use serde::Deserialize;

pub const MANIFEST_FILE: &str = "Skillnet.pkl";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestDocument {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u64,
    #[serde(rename = "defaultDependencies", default)]
    pub default_dependencies: Vec<String>,
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
    let source = fs::read_to_string(&path)
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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create Pkl evaluation runtime")?;
    let value = runtime
        .block_on(evaluator.eval_source(&source, manifest_path))
        .map_err(|error| anyhow::anyhow!("failed to evaluate {path}: {error}"))?;
    let document: ManifestDocument = serde_json::from_value(value.to_json())
        .with_context(|| format!("invalid evaluated Skillnet manifest {path}"))?;
    if document.schema_version != 1 {
        bail!(
            "unsupported Skillnet manifest schemaVersion {} in {path}; expected 1",
            document.schema_version
        );
    }
    Ok(Some(Manifest {
        document,
        path: path.to_path_buf(),
    }))
}

fn default_role() -> String {
    "entrypoint".to_string()
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
    fn read_to_string<'a>(&'a mut self, path: &'a Path) -> BoxFuture<'a, pklr::Result<String>> {
        Box::pin(async move {
            let path = self.confined(path)?;
            fs::read_to_string(&path).map_err(|error| pklr::Error::Io(path, error))
        })
    }

    fn path_exists<'a>(&'a mut self, path: &'a Path) -> BoxFuture<'a, pklr::Result<bool>> {
        Box::pin(async move { Ok(self.confined(path).is_ok()) })
    }

    fn canonicalize<'a>(&'a mut self, path: &'a Path) -> BoxFuture<'a, pklr::Result<PathBuf>> {
        Box::pin(async move { self.confined(path) })
    }

    fn read_env<'a>(&'a mut self, name: &'a str) -> BoxFuture<'a, pklr::Result<Option<String>>> {
        Box::pin(async move {
            Err(pklr::Error::Unsupported(format!(
                "environment access is disabled while evaluating Skillnet.pkl: {name}"
            )))
        })
    }

    fn fetch_text<'a>(&'a mut self, url: &'a str) -> BoxFuture<'a, pklr::Result<String>> {
        Box::pin(async move {
            Err(pklr::Error::Unsupported(format!(
                "network access is disabled while evaluating Skillnet.pkl: {url}"
            )))
        })
    }

    fn fetch_bytes<'a>(&'a mut self, url: &'a str) -> BoxFuture<'a, pklr::Result<Vec<u8>>> {
        Box::pin(async move {
            Err(pklr::Error::Unsupported(format!(
                "network access is disabled while evaluating Skillnet.pkl: {url}"
            )))
        })
    }

    fn temp_dir<'a>(&'a mut self, prefix: &'a str) -> BoxFuture<'a, pklr::Result<PathBuf>> {
        Box::pin(async move {
            Err(pklr::Error::Unsupported(format!(
                "temporary directories are disabled while evaluating Skillnet.pkl: {prefix}"
            )))
        })
    }

    fn glob<'a>(
        &'a mut self,
        _base: &'a Path,
        pattern: &'a str,
    ) -> BoxFuture<'a, pklr::Result<Vec<PathBuf>>> {
        Box::pin(async move {
            Err(pklr::Error::Unsupported(format!(
                "glob access is disabled while evaluating Skillnet.pkl: {pattern}"
            )))
        })
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
        assert!(manifest.document.default_dependencies.is_empty());
        assert_eq!(
            manifest.document.skills["fix-loop"].dependencies,
            vec!["fix-loop-ref"]
        );
        assert_eq!(manifest.document.skills["fix-loop-ref"].role, "reference");
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
}
