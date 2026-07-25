use std::{collections::BTreeMap, env, fs, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::calibration::Db;
use crate::link::{resolve_link_strategy, LinkStrategy};
use crate::model::{Target, TargetScope, ViewTarget};

pub const PROJECT_CANONICAL_REL: &str = ".skills";
pub const DEFAULT_PROJECT_WORKING_COPY_REL: &str = ".agents/skills";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub global: GlobalConfig,
    /// Optional principal used to filter manifest-declared skill access.
    /// Configurations without this field retain unrestricted legacy behavior
    /// unless a manifest requires an access selector.
    #[serde(default)]
    pub user: Option<String>,
    /// Runtime data directory for generated bundles and local state.
    #[serde(default)]
    pub data_dir: Option<String>,
    pub skills_root: Option<String>,
    pub mirror_root: Option<String>,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub link_strategy: Option<LinkStrategy>,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    #[serde(default)]
    pub subscriptions: BTreeMap<String, SubscriptionConfig>,
    /// Immutable, flake-provided Skillnet manifests merged into generated views.
    #[serde(default, rename = "external_manifests", alias = "externalManifests")]
    pub external_manifests: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Sqlite,
    #[default]
    Postgres,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub backend: DatabaseBackend,
    pub path: Option<String>,
    pub url: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            backend: DatabaseBackend::Postgres,
            path: None,
            url: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbTarget {
    Sqlite(PathBuf),
    Postgres(String),
}

#[derive(Clone, Debug, Default)]
pub struct DbOverrides {
    pub database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewConfig {
    pub label: String,
    pub path: String,
    #[serde(default)]
    pub scope: ViewScope,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewScope {
    #[default]
    Global,
    Project,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    /// Where the canonical mirror lives. Defaults to <mirror_root>/global.
    #[serde(default)]
    pub canonical_path: Option<String>,
    /// View destinations populated by `skillnet view sync`.
    pub views: Vec<ViewConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
    pub path: String,
    /// Optional repository origin used by `skillnet project clone --all`.
    #[serde(default)]
    pub origin: Option<String>,
    /// Optional link strategy override for this project scope.
    #[serde(default)]
    pub link_strategy: Option<LinkStrategy>,
    /// Relative path inside the project repo holding the generated working copy.
    #[serde(default = "default_canonical_rel")]
    pub canonical_rel: String,
    /// In-repo views to materialise via `project sync`.
    #[serde(default = "default_project_views")]
    pub views: Vec<ProjectViewConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ProjectViewConfig {
    pub rel: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionConfig {
    pub url: String,
    #[serde(default = "default_subscription_ref", rename = "ref")]
    pub ref_name: String,
    pub target: String,
    #[serde(default = "default_subscription_source")]
    pub source: String,
    #[serde(default, alias = "deletePolicy")]
    pub delete_policy: SubscriptionDeletePolicy,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SubscriptionDeletePolicy {
    #[default]
    Keep,
    Prune,
}

fn default_canonical_rel() -> String {
    DEFAULT_PROJECT_WORKING_COPY_REL.into()
}

fn default_project_views() -> Vec<ProjectViewConfig> {
    vec![ProjectViewConfig {
        rel: ".claude/skills".into(),
        label: Some("claude".into()),
    }]
}

fn default_subscription_ref() -> String {
    "main".into()
}

fn default_subscription_source() -> String {
    "global_skills".into()
}

pub fn default_data_dir() -> PathBuf {
    for var in ["skillnet_DATA_DIR", "SKILLNET_DATA_DIR"] {
        if let Some(dir) = env::var_os(var) {
            return PathBuf::from(dir);
        }
    }

    xdg_data_home().join("skillnet")
}

fn label_from_rel(rel: &str) -> String {
    rel.trim_matches('/')
        .rsplit('/')
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or(rel)
        .trim_start_matches('.')
        .to_string()
}

fn reject_legacy_schema(text: &str, path: &Utf8Path) -> Result<()> {
    const LEGACY_FIELDS: &[&str] = &[
        "sources",
        "sync_paths",
        "stale_codex_skill_paths",
        "project_source_rules",
        "extra_sources",
    ];

    let value: toml::Value =
        toml::from_str(text).with_context(|| format!("failed to parse config file {path}"))?;
    let mut found = Vec::new();
    collect_legacy_fields(&value, LEGACY_FIELDS, &mut found);
    found.sort();
    found.dedup();

    if found.is_empty() {
        return Ok(());
    }

    let primary = found.first().cloned().unwrap_or("unknown");
    bail!(
        "skillnet.toml uses the pre-Option-B schema (field `{primary}` found).\n\
These fields were removed in skillnet 0.5.0. See the migration guide for the new `views` / `canonical_rel` schema:\n\
  https://codeberg.org/caniko/skillnet/blob/main/docs/src/migration/option-b.md\n\
Fields encountered: {}\n\
File: {path}",
        found.join(", ")
    )
}

fn collect_legacy_fields(
    value: &toml::Value,
    legacy_fields: &'static [&'static str],
    found: &mut Vec<&'static str>,
) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                if let Some(field) = legacy_fields.iter().copied().find(|field| key == field) {
                    found.push(field);
                }
                collect_legacy_fields(value, legacy_fields, found);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                collect_legacy_fields(value, legacy_fields, found);
            }
        }
        _ => {}
    }
}

impl Config {
    pub fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {path}"))?;
        reject_legacy_schema(&text, path)?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("failed to parse config file {path}"))?;
        if config
            .user
            .as_deref()
            .is_some_and(|user| user.trim().is_empty())
        {
            anyhow::bail!("configured Skillnet user in {path} must not be empty");
        }
        Ok(config)
    }

    pub fn load_database_or_default(path: &Utf8Path) -> Result<DatabaseConfig> {
        if !path.exists() {
            return Ok(DatabaseConfig::default());
        }
        Ok(Self::load(path)?.database)
    }

    #[allow(dead_code)]
    pub fn resolve_db(&self) -> Result<DbTarget> {
        self.database.resolve_db()
    }

    #[allow(dead_code)]
    pub fn resolve_db_with_overrides(&self, overrides: &DbOverrides) -> Result<DbTarget> {
        self.database.resolve_db_with_overrides(overrides)
    }

    pub fn targets(&self, mirror_root: &Utf8Path) -> Result<Vec<Target>> {
        self.targets_with_link_override(mirror_root, None)
    }

    pub fn targets_with_link_override(
        &self,
        mirror_root: &Utf8Path,
        cli_link_strategy: Option<LinkStrategy>,
    ) -> Result<Vec<Target>> {
        let mut targets = Vec::with_capacity(self.projects.len() + 1);
        targets.push(self.global_target_with_link_override(mirror_root, cli_link_strategy)?);
        for project in &self.projects {
            targets.push(self.project_target_with_link_override(
                mirror_root,
                project,
                cli_link_strategy,
            )?);
        }
        Ok(targets)
    }

    pub fn global_target(&self, mirror_root: &Utf8Path) -> Result<Target> {
        self.global_target_with_link_override(mirror_root, None)
    }

    pub fn global_target_with_link_override(
        &self,
        mirror_root: &Utf8Path,
        cli_link_strategy: Option<LinkStrategy>,
    ) -> Result<Target> {
        let canonical_path = match self.global.canonical_path.as_deref() {
            Some(path) => expand_path(path)?,
            None => mirror_root.join("global"),
        };
        Ok(Target {
            name: "global".to_string(),
            scope: TargetScope::Global,
            link_strategy: resolve_link_strategy(
                TargetScope::Global,
                self,
                None,
                cli_link_strategy,
            ),
            canonical_path,
            views: self
                .global
                .views
                .iter()
                .filter(|view| view.scope == ViewScope::Global)
                .map(|view| {
                    Ok(ViewTarget {
                        label: view.label.clone(),
                        path: expand_path(&view.path)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            aggregator_path: None,
            project_root: None,
            canonical_rel: None,
            origin: None,
        })
    }

    pub fn project_target(
        &self,
        mirror_root: &Utf8Path,
        project: &ProjectConfig,
    ) -> Result<Target> {
        self.project_target_with_link_override(mirror_root, project, None)
    }

    pub fn project_target_with_link_override(
        &self,
        _mirror_root: &Utf8Path,
        project: &ProjectConfig,
        cli_link_strategy: Option<LinkStrategy>,
    ) -> Result<Target> {
        let project_root = expand_path(&project.path)?;
        reject_project_view_canonical_overlap(project)?;
        let canonical_path = project_root.join(PROJECT_CANONICAL_REL);
        let aggregator_path = (normalize_project_rel(&project.canonical_rel)
            != PROJECT_CANONICAL_REL)
            .then(|| project_root.join(&project.canonical_rel));

        Ok(Target {
            name: project.name.clone(),
            scope: TargetScope::Project,
            link_strategy: resolve_link_strategy(
                TargetScope::Project,
                self,
                Some(project),
                cli_link_strategy,
            ),
            canonical_path,
            views: project
                .views
                .iter()
                .map(|view| {
                    Ok(ViewTarget {
                        label: view
                            .label
                            .clone()
                            .unwrap_or_else(|| label_from_rel(&view.rel)),
                        path: project_root.join(&view.rel),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            aggregator_path,
            project_root: Some(project_root),
            canonical_rel: Some(project.canonical_rel.clone()),
            origin: project.origin.clone(),
        })
    }
}

pub fn legacy_project_canonical_warning(target: &Target) -> Option<String> {
    if !target.canonical_path.is_dir() {
        Some(format!(
            "project {}: canonical '.skills' store is missing at {}; create it or run `skillnet skill new {}/<name>`.",
            target.name,
            target.canonical_path,
            target.name
        ))
    } else {
        None
    }
}

fn reject_project_view_canonical_overlap(project: &ProjectConfig) -> Result<()> {
    let canonical = normalize_project_rel(PROJECT_CANONICAL_REL);
    let working_copy = normalize_project_rel(&project.canonical_rel);
    for view in &project.views {
        let view_rel = normalize_project_rel(&view.rel);
        if view_rel == canonical {
            bail!(
                "project `{}` view rel `{}` equals canonical skill store `.skills`; remove it from views",
                project.name,
                view.rel
            );
        }
        if view_rel == working_copy {
            bail!(
                "project `{}` view rel `{}` equals working-copy rel `{}`; remove it from views or choose a different canonical_rel",
                project.name,
                view.rel,
                project.canonical_rel
            );
        }
    }
    Ok(())
}

fn normalize_project_rel(rel: &str) -> &str {
    rel.trim_matches('/')
}

impl DatabaseConfig {
    #[allow(dead_code)]
    pub fn resolve_db(&self) -> Result<DbTarget> {
        self.resolve_db_with_overrides(&DbOverrides::default())
    }

    pub fn resolve_db_with_overrides(&self, overrides: &DbOverrides) -> Result<DbTarget> {
        if let Some(url) = non_empty(overrides.database_url.as_deref()) {
            warn_url_wins_over_data_dir();
            return Ok(DbTarget::Postgres(url.to_string()));
        }

        if let Some(url) = env_database_url() {
            warn_url_wins_over_data_dir();
            return Ok(DbTarget::Postgres(url));
        }

        if let Some(url) = non_empty(self.url.as_deref()) {
            warn_url_wins_over_data_dir();
            return Ok(DbTarget::Postgres(url.to_string()));
        }

        if let Some(path) = env_data_dir_path() {
            return Ok(DbTarget::Sqlite(path));
        }

        if self.backend == DatabaseBackend::Postgres {
            let url = non_empty(self.url.as_deref()).ok_or_else(|| {
                anyhow!(
                    "database.backend = \"postgres\" requires database.url, \
                     SKILLNET_DATABASE_URL, SKILLNET_DB_URL, DATABASE_URL, or --database-url"
                )
            })?;
            warn_url_wins_over_data_dir();
            return Ok(DbTarget::Postgres(url.to_string()));
        }

        if let Some(path) = non_empty(self.path.as_deref()) {
            return Ok(DbTarget::Sqlite(expand_path(path)?.into_std_path_buf()));
        }

        Ok(DbTarget::Sqlite(Db::default_path()))
    }
}

fn env_database_url() -> Option<String> {
    ["SKILLNET_DATABASE_URL", "SKILLNET_DB_URL", "DATABASE_URL"]
        .into_iter()
        .find_map(|var| env::var(var).ok().and_then(non_empty_owned))
}

fn env_data_dir_path() -> Option<PathBuf> {
    ["skillnet_DATA_DIR", "SKILLNET_DATA_DIR"]
        .into_iter()
        .find_map(|var| env::var_os(var).map(PathBuf::from))
        .map(|dir| dir.join("multi-phase-plan").join("calibration.sqlite"))
}

fn warn_url_wins_over_data_dir() {
    if ["skillnet_DATA_DIR", "SKILLNET_DATA_DIR"]
        .into_iter()
        .any(|var| env::var_os(var).is_some())
    {
        eprintln!(
            "warning: database URL override is set; ignoring SKILLNET_DATA_DIR/skillnet_DATA_DIR"
        );
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn expand_path(raw: &str) -> Result<Utf8PathBuf> {
    let raw = expand_env_vars(raw)?;
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    let path = Utf8PathBuf::from(raw.as_str());
    if path.is_absolute() {
        Ok(path)
    } else {
        env::current_dir()
            .map_err(anyhow::Error::from)
            .and_then(|p| Utf8PathBuf::from_path_buf(p).map_err(|_| anyhow!("cwd is not UTF-8")))
            .map(|cwd| cwd.join(path))
    }
}

fn expand_env_vars(raw: &str) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('{') => {
                chars.next();
                let mut name = String::new();
                for next in chars.by_ref() {
                    if next == '}' {
                        break;
                    }
                    name.push(next);
                }
                if name.is_empty() {
                    bail!("empty environment variable expansion in path `{raw}`");
                }
                out.push_str(&env::var(&name).with_context(|| {
                    format!("environment variable `{name}` is not set for path `{raw}`")
                })?);
            }
            Some(next) if is_env_name_start(next) => {
                let mut name = String::new();
                while let Some(next) = chars.peek().copied() {
                    if !is_env_name_char(next) {
                        break;
                    }
                    name.push(next);
                    chars.next();
                }
                out.push_str(&env::var(&name).with_context(|| {
                    format!("environment variable `{name}` is not set for path `{raw}`")
                })?);
            }
            _ => out.push('$'),
        }
    }
    Ok(out)
}

fn is_env_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_env_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub fn default_config_path() -> Result<Utf8PathBuf> {
    default_xdg_config_path("skillnet.toml")
}

pub fn default_catalog_config_path() -> Result<Utf8PathBuf> {
    default_xdg_config_path("skillnet.catalog.toml")
}

pub fn legacy_config_path() -> Utf8PathBuf {
    Utf8PathBuf::from("skillnet.toml")
}

pub fn legacy_catalog_config_path() -> Utf8PathBuf {
    Utf8PathBuf::from("skillnet.catalog.toml")
}

pub fn config_is_hm_managed(path: &Utf8Path) -> bool {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    Utf8PathBuf::from_path_buf(canonical)
        .map(|path| path.as_str().starts_with("/nix/store/"))
        .unwrap_or(false)
}

pub fn hm_managed_error_message(path: &Utf8Path, file_name: &str) -> String {
    format!(
        "{file_name} at {path} is managed by Home Manager (read-only).\n\
         hint: edit programs.skillnet.settings in your Home Manager configuration,\n\
               then run `home-manager switch`."
    )
}

fn default_xdg_config_path(file_name: &str) -> Result<Utf8PathBuf> {
    let config_home = match non_empty(env::var("XDG_CONFIG_HOME").ok().as_deref()) {
        Some(path) => Utf8PathBuf::from(path),
        None => home_dir()?.join(".config"),
    };
    Ok(config_home.join("skillnet").join(file_name))
}

fn xdg_data_home() -> PathBuf {
    if let Some(dir) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir);
    }

    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share"))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
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
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    #[test]
    fn expands_home_prefix() {
        let home = env::var("HOME").unwrap();
        let path = expand_path("~/x/y").unwrap();
        assert_eq!(path, Utf8PathBuf::from(home).join("x/y"));
    }

    #[test]
    fn database_table_rejects_unknown_keys() {
        let err = toml::from_str::<Config>(
            r#"
[global]
views = []

[database]
backend = "sqlite"
bogus = true
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn accepts_explicit_runtime_data_dir() {
        let config: Config = toml::from_str(
            r#"
data_dir = "/var/lib/skillnet"

[global]
views = []
"#,
        )
        .unwrap();
        assert_eq!(config.data_dir.as_deref(), Some("/var/lib/skillnet"));
    }

    #[test]
    fn accepts_a_configured_user() {
        let config: Config = toml::from_str(
            r#"
user = "dejana"

[global]
views = []
"#,
        )
        .unwrap();
        assert_eq!(config.user.as_deref(), Some("dejana"));
    }

    #[test]
    fn load_rejects_an_empty_configured_user() {
        let error = load_config_from_text(
            r#"
user = "  "

[global]
views = []
"#,
        );
        assert!(error.to_string().contains("configured Skillnet user"));
    }

    #[test]
    fn load_rejects_legacy_global_sources_schema() {
        let err = load_config_from_text(
            r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []
"#,
        );

        let message = err.to_string();
        assert!(message.contains("pre-Option-B schema"));
        assert!(message.contains("sources"));
        assert!(message.contains("sync_paths"));
        assert!(message.contains("stale_codex_skill_paths"));
        assert!(message.contains("docs/src/migration/option-b.md"));
        assert!(message.contains("File:"));
    }

    #[test]
    fn load_rejects_legacy_root_project_source_rules_schema() {
        let err = load_config_from_text(
            r#"
project_source_rules = []

[global]
views = []
"#,
        );

        let message = err.to_string();
        assert!(message.contains("pre-Option-B schema"));
        assert!(message.contains("project_source_rules"));
    }

    #[test]
    fn load_rejects_legacy_project_extra_sources_schema() {
        let err = load_config_from_text(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
extra_sources = []
"#,
        );

        let message = err.to_string();
        assert!(message.contains("pre-Option-B schema"));
        assert!(message.contains("extra_sources"));
    }

    #[test]
    fn project_target_defaults_to_canonical_rel_and_standard_views() {
        let cfg = toml::from_str::<Config>(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
"#,
        )
        .unwrap();

        let target = cfg
            .project_target(
                Utf8Path::new("/tmp/mirror"),
                cfg.projects.first().expect("project fixture"),
            )
            .unwrap();
        assert_eq!(
            target.canonical_path,
            Utf8PathBuf::from("/tmp/demo/.skills")
        );
        assert_eq!(
            target.aggregator_path,
            Some(Utf8PathBuf::from("/tmp/demo/.agents/skills"))
        );
        assert_eq!(target.views.len(), 1);
        assert_eq!(target.views[0].label, "claude");
        assert_eq!(target.views[0].path, "/tmp/demo/.claude/skills");
    }

    #[test]
    fn project_target_rejects_view_matching_canonical_rel() {
        let cfg = toml::from_str::<Config>(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
canonical_rel = ".agents/skills"
views = [{ rel = ".agents/skills", label = "agents" }]
"#,
        )
        .unwrap();

        let err = cfg
            .project_target(
                Utf8Path::new("/tmp/mirror"),
                cfg.projects.first().expect("project fixture"),
            )
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("view rel `.agents/skills` equals working-copy rel `.agents/skills`")
        );
    }

    #[test]
    fn project_target_honors_explicit_legacy_canonical_rel() {
        let cfg = toml::from_str::<Config>(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
canonical_rel = ".skills"
"#,
        )
        .unwrap();

        let target = cfg
            .project_target(
                Utf8Path::new("/tmp/mirror"),
                cfg.projects.first().expect("project fixture"),
            )
            .unwrap();
        assert_eq!(
            target.canonical_path,
            Utf8PathBuf::from("/tmp/demo/.skills")
        );
        assert_eq!(target.aggregator_path, None);
        assert_eq!(target.canonical_rel.as_deref(), Some(".skills"));
    }

    #[test]
    fn link_strategy_resolution_precedence_covers_all_rungs() {
        let defaults = toml::from_str::<Config>(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
"#,
        )
        .unwrap();
        let project = defaults.projects.first().expect("project fixture");
        assert_eq!(
            resolve_link_strategy(TargetScope::Global, &defaults, None, None),
            LinkStrategy::Symlink
        );
        assert_eq!(
            resolve_link_strategy(TargetScope::Project, &defaults, Some(project), None),
            LinkStrategy::Hardlink
        );

        let top_level = toml::from_str::<Config>(
            r#"
link_strategy = "symlink"

[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
"#,
        )
        .unwrap();
        let project = top_level.projects.first().expect("project fixture");
        assert_eq!(
            resolve_link_strategy(TargetScope::Project, &top_level, Some(project), None),
            LinkStrategy::Symlink
        );

        let per_project = toml::from_str::<Config>(
            r#"
link_strategy = "hardlink"

[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
link_strategy = "symlink"
"#,
        )
        .unwrap();
        let project = per_project.projects.first().expect("project fixture");
        assert_eq!(
            resolve_link_strategy(TargetScope::Project, &per_project, Some(project), None),
            LinkStrategy::Symlink
        );
        assert_eq!(
            resolve_link_strategy(
                TargetScope::Project,
                &per_project,
                Some(project),
                Some(LinkStrategy::Hardlink),
            ),
            LinkStrategy::Hardlink
        );
    }

    #[test]
    fn link_strategy_config_fields_parse_and_reject_invalid_values() {
        let temp = tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(temp.path().join("skillnet.toml")).unwrap();
        fs::write(
            &path,
            r#"
link_strategy = "hardlink"

[global]
views = []

[[projects]]
name = "demo"
path = "/tmp/demo"
link_strategy = "symlink"
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.link_strategy, Some(LinkStrategy::Hardlink));
        assert_eq!(
            cfg.projects.first().unwrap().link_strategy,
            Some(LinkStrategy::Symlink)
        );

        fs::write(
            &path,
            r#"
link_strategy = "soft"

[global]
views = []
"#,
        )
        .unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(err.to_string().contains("failed to parse config file"));
    }

    #[test]
    fn detects_hm_managed_config_paths() {
        assert!(config_is_hm_managed(Utf8Path::new(
            "/nix/store/abc/skillnet.toml"
        )));
        assert!(!config_is_hm_managed(Utf8Path::new(
            "/home/x/.config/skillnet/skillnet.toml"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn detects_hm_managed_config_after_canonicalizing_symlink() {
        use std::os::unix::fs as unix_fs;

        let temp = tempdir().unwrap();
        let store_target =
            Utf8Path::new("/nix/store")
                .read_dir_utf8()
                .ok()
                .and_then(|mut entries| {
                    entries.find_map(|entry| {
                        let path = entry.ok()?.path().join("skillnet.toml");
                        path.is_file().then_some(path)
                    })
                });
        let Some(store_target) = store_target else {
            return;
        };
        let link = temp.path().join("skillnet.toml");
        unix_fs::symlink(store_target.as_std_path(), &link).unwrap();
        let link = Utf8PathBuf::from_path_buf(link).unwrap();

        assert!(config_is_hm_managed(&link));
    }

    #[test]
    fn db_resolution_precedence_covers_all_rungs() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempdir().unwrap();
        let _env = EnvSnapshot::capture();
        clear_db_env();
        env::set_var("XDG_DATA_HOME", temp.path().join("xdg"));

        let cfg = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            path: Some(temp.path().join("config.sqlite").display().to_string()),
            url: Some("postgres://config".to_string()),
        };

        env::set_var("skillnet_DATA_DIR", temp.path().join("data"));
        env::set_var("SKILLNET_DATABASE_URL", "postgres://env");
        assert_eq!(
            cfg.resolve_db_with_overrides(&DbOverrides {
                database_url: Some("postgres://cli".to_string()),
            })
            .unwrap(),
            DbTarget::Postgres("postgres://cli".to_string())
        );

        assert_eq!(
            cfg.resolve_db().unwrap(),
            DbTarget::Postgres("postgres://env".to_string())
        );

        env::remove_var("SKILLNET_DATABASE_URL");
        env::set_var("SKILLNET_DB_URL", "postgres://short-env");
        assert_eq!(
            cfg.resolve_db().unwrap(),
            DbTarget::Postgres("postgres://short-env".to_string())
        );

        env::remove_var("SKILLNET_DB_URL");
        env::set_var("DATABASE_URL", "postgres://database-url-env");
        assert_eq!(
            cfg.resolve_db().unwrap(),
            DbTarget::Postgres("postgres://database-url-env".to_string())
        );

        env::remove_var("DATABASE_URL");
        assert_eq!(
            cfg.resolve_db().unwrap(),
            DbTarget::Postgres("postgres://config".to_string())
        );

        let sqlite_cfg = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            path: Some(temp.path().join("config.sqlite").display().to_string()),
            url: None,
        };
        assert_eq!(
            sqlite_cfg.resolve_db().unwrap(),
            DbTarget::Sqlite(
                temp.path()
                    .join("data")
                    .join("multi-phase-plan")
                    .join("calibration.sqlite")
            )
        );

        env::remove_var("skillnet_DATA_DIR");
        assert_eq!(
            sqlite_cfg.resolve_db().unwrap(),
            DbTarget::Sqlite(temp.path().join("config.sqlite"))
        );

        let default_sqlite_cfg = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            path: None,
            url: None,
        };
        assert_eq!(
            default_sqlite_cfg.resolve_db().unwrap(),
            DbTarget::Sqlite(
                temp.path()
                    .join("xdg")
                    .join("skillnet")
                    .join("multi-phase-plan")
                    .join("calibration.sqlite")
            )
        );

        let default_cfg = DatabaseConfig::default();
        let err = default_cfg.resolve_db().unwrap_err();
        assert!(err
            .to_string()
            .contains("database.backend = \"postgres\" requires database.url"));
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn clear_db_env() {
        for key in [
            "SKILLNET_DATABASE_URL",
            "SKILLNET_DB_URL",
            "DATABASE_URL",
            "skillnet_DATA_DIR",
            "SKILLNET_DATA_DIR",
            "XDG_DATA_HOME",
        ] {
            env::remove_var(key);
        }
    }

    fn load_config_from_text(text: &str) -> anyhow::Error {
        let temp = tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(temp.path().join("skillnet.toml")).unwrap();
        fs::write(&path, text).unwrap();
        Config::load(&path).unwrap_err()
    }

    struct EnvSnapshot {
        values: Vec<(&'static str, Option<String>)>,
    }

    impl EnvSnapshot {
        fn capture() -> Self {
            Self {
                values: [
                    "SKILLNET_DATABASE_URL",
                    "SKILLNET_DB_URL",
                    "DATABASE_URL",
                    "skillnet_DATA_DIR",
                    "SKILLNET_DATA_DIR",
                    "XDG_DATA_HOME",
                ]
                .into_iter()
                .map(|key| (key, env::var(key).ok()))
                .collect(),
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
        }
    }
}
