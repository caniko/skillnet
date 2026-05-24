use std::{env, fs, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::calibration::Db;
use crate::model::{Source, Target};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub project_source_rules: Vec<ProjectSourceRule>,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
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
                     SKILLNET_DATABASE_URL, SKILLNET_DB_URL, or --database-url"
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
    ["SKILLNET_DATABASE_URL", "SKILLNET_DB_URL"]
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
sources = []
sync_paths = []
stale_codex_skill_paths = []

[database]
backend = "sqlite"
bogus = true
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
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
            "skillnet_DATA_DIR",
            "SKILLNET_DATA_DIR",
            "XDG_DATA_HOME",
        ] {
            env::remove_var(key);
        }
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
