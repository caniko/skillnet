use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use rusqlite::Connection;

const DB_FILE: &str = "calibration.sqlite";
const LEGACY_REPO_DATA_DIR: &str = "data/multi-phase-plan";
const SKILL_NAME: &str = "multi-phase-plan";
const MIGRATIONS: &[(i64, &str, &str)] = &[(
    1,
    "001-initial.sql",
    include_str!("../../data/multi-phase-plan/schema/001-initial.sql"),
)];

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database parent {}", parent.display())
            })?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open sqlite database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable sqlite WAL journal mode")?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable sqlite foreign keys")?;

        let mut db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn default_path() -> PathBuf {
        if let Some(dir) = std::env::var_os("SKILLNET_DATA_DIR") {
            return PathBuf::from(dir).join(SKILL_NAME).join(DB_FILE);
        }

        if let Some(repo_root) = std::env::var_os("AI_SKILLS_REPO") {
            return PathBuf::from(repo_root)
                .join(LEGACY_REPO_DATA_DIR)
                .join(DB_FILE);
        }

        xdg_data_home()
            .join("skillnet")
            .join(SKILL_NAME)
            .join(DB_FILE)
    }

    #[allow(dead_code)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    fn migrate(&mut self) -> anyhow::Result<()> {
        let migrations = load_migrations()?;
        let applied = self.applied_versions()?;

        for migration in migrations {
            if applied.contains(&migration.version) {
                continue;
            }

            let tx = self.conn.transaction().with_context(|| {
                format!("failed to start migration {} transaction", migration.name)
            })?;
            tx.execute_batch(&migration.sql)
                .with_context(|| format!("failed to apply migration {}", migration.name))?;
            tx.execute(
                "INSERT INTO schema_versions (version, applied_at) VALUES (?1, ?2)",
                (migration.version, unix_timestamp()?),
            )
            .with_context(|| format!("failed to record migration {}", migration.name))?;
            tx.commit()
                .with_context(|| format!("failed to commit migration {}", migration.name))?;
        }

        Ok(())
    }

    fn applied_versions(&self) -> anyhow::Result<HashSet<i64>> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = 'schema_versions'
            )",
            [],
            |row| row.get(0),
        )?;

        if !exists {
            return Ok(HashSet::new());
        }

        let mut stmt = self
            .conn
            .prepare("SELECT version FROM schema_versions")
            .context("failed to prepare schema_versions query")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .context("failed to query schema_versions")?;
        let mut versions = HashSet::new();
        for row in rows {
            versions.insert(row.context("failed to read schema version")?);
        }
        Ok(versions)
    }
}

struct Migration {
    version: i64,
    name: String,
    sql: String,
}

fn load_migrations() -> anyhow::Result<Vec<Migration>> {
    MIGRATIONS
        .iter()
        .map(|(version, name, sql)| {
            parse_migration_version(name)?;
            Ok(Migration {
                version: *version,
                name: (*name).to_string(),
                sql: (*sql).to_string(),
            })
        })
        .collect()
}

fn parse_migration_version(name: &str) -> anyhow::Result<i64> {
    if name.len() < "001-a.sql".len()
        || !name.ends_with(".sql")
        || name.as_bytes().get(3) != Some(&b'-')
        || !name.as_bytes()[..3].iter().all(u8::is_ascii_digit)
    {
        bail!("migration filename must match NNN-<desc>.sql: {name}");
    }

    let desc = &name[4..name.len() - 4];
    if desc.is_empty() {
        bail!("migration filename must include a description: {name}");
    }

    name[..3]
        .parse()
        .with_context(|| format!("failed to parse migration version from {name}"))
}

fn xdg_data_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir);
    }

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/share"))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}
