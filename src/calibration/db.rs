use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use rusqlite::{
    params_from_iter,
    types::{Value, ValueRef},
    Connection,
};

#[cfg(feature = "postgres")]
#[path = "db_postgres.rs"]
mod db_postgres;

// Calibration SQL uses PostgreSQL-style `$N` placeholders at call sites.
// The SQLite backend rewrites them to `?N` immediately before execution.

const DB_FILE: &str = "calibration.sqlite";
const SKILL_NAME: &str = "multi-phase-plan";
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "001-initial.sql",
        include_str!("../../data/multi-phase-plan/schema/001-initial.sql"),
    ),
    (
        2,
        "002-heuristic-thresholds.sql",
        include_str!("../../data/multi-phase-plan/schema/002-heuristic-thresholds.sql"),
    ),
    (
        3,
        "003-skill-invocations.sql",
        include_str!("../../data/multi-phase-plan/schema/003-skill-invocations.sql"),
    ),
];
#[cfg(feature = "postgres")]
const POSTGRES_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "001-initial.sql",
        include_str!("../../data/multi-phase-plan/schema-pg/001-initial.sql"),
    ),
    (
        2,
        "002-heuristic-thresholds.sql",
        include_str!("../../data/multi-phase-plan/schema-pg/002-heuristic-thresholds.sql"),
    ),
    (
        3,
        "003-skill-invocations.sql",
        include_str!("../../data/multi-phase-plan/schema-pg/003-skill-invocations.sql"),
    ),
];

pub struct Db {
    backend: Backend,
}

enum Backend {
    Sqlite(Connection),
    #[cfg(feature = "postgres")]
    Postgres(Box<db_postgres::PostgresBackend>),
}

#[derive(Clone, Copy)]
enum BackendKind {
    Sqlite,
    #[cfg(feature = "postgres")]
    Postgres,
}

pub struct Tx<'db> {
    backend: TxBackend<'db>,
}

enum TxBackend<'db> {
    Sqlite(Option<rusqlite::Transaction<'db>>),
    #[cfg(feature = "postgres")]
    Postgres(Option<db_postgres::PostgresTransaction<'db>>),
}

#[derive(Clone, Debug)]
pub enum DbParam<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Bool(bool),
}

#[derive(Clone, Debug)]
pub struct DbRow {
    values: Vec<DbValue>,
}

#[derive(Clone, Debug)]
enum DbValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    #[cfg(feature = "postgres")]
    Bool(bool),
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

        let mut db = Db {
            backend: Backend::Sqlite(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(feature = "postgres")]
    pub fn open_postgres(url: &str) -> anyhow::Result<Db> {
        let backend = db_postgres::PostgresBackend::connect(url)?;
        let mut db = Db {
            backend: Backend::Postgres(Box::new(backend)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn default_path() -> PathBuf {
        for var in ["skillnet_DATA_DIR", "SKILLNET_DATA_DIR"] {
            if let Some(dir) = std::env::var_os(var) {
                return PathBuf::from(dir).join(SKILL_NAME).join(DB_FILE);
            }
        }

        xdg_data_home()
            .join("skillnet")
            .join(SKILL_NAME)
            .join(DB_FILE)
    }

    pub fn execute(&self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<usize> {
        match &self.backend {
            Backend::Sqlite(conn) => sqlite_execute(conn, sql, params),
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.execute(sql, params),
        }
    }

    pub fn execute_returning_id(&self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<i64> {
        match &self.backend {
            Backend::Sqlite(conn) => {
                sqlite_execute(conn, sql, params)?;
                Ok(conn.last_insert_rowid())
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.execute_returning_id(sql, params),
        }
    }

    pub fn execute_batch(&self, sql: &str) -> anyhow::Result<()> {
        match &self.backend {
            Backend::Sqlite(conn) => conn
                .execute_batch(sql)
                .context("failed to execute SQL batch"),
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.execute_batch(sql),
        }
    }

    pub fn query_one<T>(
        &self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        self.query_optional(sql, params, map)?
            .ok_or_else(|| anyhow::anyhow!("query returned no rows"))
    }

    pub fn query_optional<T>(
        &self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Option<T>> {
        match &self.backend {
            Backend::Sqlite(conn) => sqlite_query_optional(conn, sql, params, map),
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.query_optional(sql, params, map),
        }
    }

    pub fn query_all<T>(
        &self,
        sql: &str,
        params: &[DbParam<'_>],
        mut map: impl FnMut(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Vec<T>> {
        match &self.backend {
            Backend::Sqlite(conn) => sqlite_query_all(conn, sql, params, &mut map),
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.query_all(sql, params, &mut map),
        }
    }

    pub fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        match &mut self.backend {
            Backend::Sqlite(conn) => {
                let tx = conn.transaction().context("failed to start transaction")?;
                let mut tx = Tx {
                    backend: TxBackend::Sqlite(Some(tx)),
                };
                let value = f(&mut tx)?;
                tx.commit()?;
                Ok(value)
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres(backend) => backend.transaction(|pg_tx| {
                let mut tx = Tx {
                    backend: TxBackend::Postgres(Some(pg_tx)),
                };
                let value = f(&mut tx)?;
                tx.commit()?;
                Ok(value)
            }),
        }
    }

    pub fn unix_date_expr(&self, column: &str) -> String {
        match &self.backend {
            Backend::Sqlite(_) => format!("strftime('%Y-%m-%d', {column}, 'unixepoch')"),
            #[cfg(feature = "postgres")]
            Backend::Postgres(_) => {
                format!("to_char(to_timestamp({column}) AT TIME ZONE 'UTC', 'YYYY-MM-DD')")
            }
        }
    }

    #[allow(dead_code)]
    pub fn sqlite_pragma_string(&self, name: &str) -> anyhow::Result<String> {
        match name {
            "journal_mode" | "foreign_keys" => {
                self.query_one(&format!("PRAGMA {name}"), &[], |row| row.get_string(0))
            }
            _ => bail!("unsupported sqlite pragma inspection: {name}"),
        }
    }

    fn migrate(&mut self) -> anyhow::Result<()> {
        let migrations = load_migrations(self.backend_kind())?;
        let applied = self.applied_versions()?;

        for migration in migrations {
            if applied.contains(&migration.version) {
                continue;
            }

            self.transaction(|tx| {
                tx.execute_batch(&migration.sql)
                    .with_context(|| format!("failed to apply migration {}", migration.name))?;
                tx.execute(
                    "INSERT INTO schema_versions (version, applied_at) VALUES ($1, $2)",
                    &[
                        DbParam::Integer(migration.version),
                        DbParam::Integer(unix_timestamp()?),
                    ],
                )
                .with_context(|| format!("failed to record migration {}", migration.name))?;
                Ok(())
            })
            .with_context(|| format!("failed to commit migration {}", migration.name))?;
        }

        Ok(())
    }

    fn applied_versions(&self) -> anyhow::Result<HashSet<i64>> {
        let exists_sql = match self.backend_kind() {
            BackendKind::Sqlite => {
                "SELECT EXISTS (
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'schema_versions'
                )"
            }
            #[cfg(feature = "postgres")]
            BackendKind::Postgres => {
                "SELECT EXISTS (
                    SELECT 1 FROM information_schema.tables
                    WHERE table_schema = current_schema()
                      AND table_name = 'schema_versions'
                )"
            }
        };
        let exists: bool = self.query_one(exists_sql, &[], |row| row.get_bool(0))?;

        if !exists {
            return Ok(HashSet::new());
        }

        let rows = self
            .query_all("SELECT version FROM schema_versions", &[], |row| {
                row.get_i64(0)
            })
            .context("failed to query schema_versions")?;
        let mut versions = HashSet::new();
        for version in rows {
            versions.insert(version);
        }
        Ok(versions)
    }

    fn backend_kind(&self) -> BackendKind {
        match &self.backend {
            Backend::Sqlite(_) => BackendKind::Sqlite,
            #[cfg(feature = "postgres")]
            Backend::Postgres(_) => BackendKind::Postgres,
        }
    }
}

impl<'db> Tx<'db> {
    pub fn execute(&mut self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<usize> {
        match &mut self.backend {
            TxBackend::Sqlite(tx) => sqlite_execute(
                tx.as_mut()
                    .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?,
                sql,
                params,
            ),
            #[cfg(feature = "postgres")]
            TxBackend::Postgres(tx) => tx
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .execute(sql, params),
        }
    }

    pub fn execute_batch(&mut self, sql: &str) -> anyhow::Result<()> {
        match &mut self.backend {
            TxBackend::Sqlite(tx) => tx
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .execute_batch(sql)
                .context("failed to execute SQL batch"),
            #[cfg(feature = "postgres")]
            TxBackend::Postgres(tx) => tx
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .execute_batch(sql),
        }
    }

    pub fn query_one<T>(
        &mut self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        self.query_optional(sql, params, map)?
            .ok_or_else(|| anyhow::anyhow!("query returned no rows"))
    }

    pub fn query_optional<T>(
        &mut self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Option<T>> {
        match &mut self.backend {
            TxBackend::Sqlite(tx) => sqlite_query_optional(
                tx.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?,
                sql,
                params,
                map,
            ),
            #[cfg(feature = "postgres")]
            TxBackend::Postgres(tx) => tx
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .query_optional(sql, params, map),
        }
    }

    #[allow(dead_code)]
    pub fn query_all<T>(
        &mut self,
        sql: &str,
        params: &[DbParam<'_>],
        mut map: impl FnMut(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Vec<T>> {
        match &mut self.backend {
            TxBackend::Sqlite(tx) => sqlite_query_all(
                tx.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?,
                sql,
                params,
                &mut map,
            ),
            #[cfg(feature = "postgres")]
            TxBackend::Postgres(tx) => tx
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .query_all(sql, params, &mut map),
        }
    }

    fn commit(&mut self) -> anyhow::Result<()> {
        match &mut self.backend {
            TxBackend::Sqlite(tx) => tx
                .take()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .commit()
                .context("failed to commit transaction"),
            #[cfg(feature = "postgres")]
            TxBackend::Postgres(tx) => tx
                .take()
                .ok_or_else(|| anyhow::anyhow!("transaction already committed"))?
                .commit(),
        }
    }
}

impl DbRow {
    pub fn get_i64(&self, index: usize) -> anyhow::Result<i64> {
        match self.value(index)? {
            DbValue::Integer(value) => Ok(*value),
            value => bail!("column {index} is not an integer: {value:?}"),
        }
    }

    pub fn get_optional_i64(&self, index: usize) -> anyhow::Result<Option<i64>> {
        match self.value(index)? {
            DbValue::Null => Ok(None),
            DbValue::Integer(value) => Ok(Some(*value)),
            value => bail!("column {index} is not a nullable integer: {value:?}"),
        }
    }

    pub fn get_f64(&self, index: usize) -> anyhow::Result<f64> {
        match self.value(index)? {
            DbValue::Real(value) => Ok(*value),
            DbValue::Integer(value) => Ok(*value as f64),
            value => bail!("column {index} is not a real number: {value:?}"),
        }
    }

    pub fn get_bool(&self, index: usize) -> anyhow::Result<bool> {
        match self.value(index)? {
            DbValue::Integer(value) => Ok(*value != 0),
            #[cfg(feature = "postgres")]
            DbValue::Bool(value) => Ok(*value),
            value => bail!("column {index} is not a boolean-compatible value: {value:?}"),
        }
    }

    pub fn get_string(&self, index: usize) -> anyhow::Result<String> {
        match self.value(index)? {
            DbValue::Text(value) => Ok(value.clone()),
            value => bail!("column {index} is not text: {value:?}"),
        }
    }

    pub fn get_optional_string(&self, index: usize) -> anyhow::Result<Option<String>> {
        match self.value(index)? {
            DbValue::Null => Ok(None),
            DbValue::Text(value) => Ok(Some(value.clone())),
            value => bail!("column {index} is not nullable text: {value:?}"),
        }
    }

    fn value(&self, index: usize) -> anyhow::Result<&DbValue> {
        self.values
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("column index {index} out of bounds"))
    }
}

impl<'a> DbParam<'a> {
    pub fn nullable_text(value: Option<&'a str>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }

    pub fn nullable_i64(value: Option<i64>) -> Self {
        value.map_or(Self::Null, Self::Integer)
    }
}

impl<'a> From<i64> for DbParam<'a> {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl<'a> From<f64> for DbParam<'a> {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}

impl<'a> From<bool> for DbParam<'a> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl<'a> From<&'a str> for DbParam<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(value)
    }
}

impl<'a> From<&'a String> for DbParam<'a> {
    fn from(value: &'a String) -> Self {
        Self::Text(value)
    }
}

fn sqlite_execute(conn: &Connection, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<usize> {
    let sql = sqlite_placeholders(sql);
    let values = sqlite_params(params);
    conn.execute(sql.as_ref(), params_from_iter(values.iter()))
        .with_context(|| format!("failed to execute SQL: {sql}"))
}

fn sqlite_query_optional<T>(
    conn: &Connection,
    sql: &str,
    params: &[DbParam<'_>],
    map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
) -> anyhow::Result<Option<T>> {
    let sql = sqlite_placeholders(sql);
    let values = sqlite_params(params);
    let mut stmt = conn
        .prepare(sql.as_ref())
        .with_context(|| format!("failed to prepare SQL: {sql}"))?;
    let mut rows = stmt
        .query(params_from_iter(values.iter()))
        .with_context(|| format!("failed to query SQL: {sql}"))?;
    let Some(row) = rows
        .next()
        .with_context(|| format!("failed to fetch row for SQL: {sql}"))?
    else {
        return Ok(None);
    };
    let row = sqlite_row(row)?;
    if rows
        .next()
        .with_context(|| format!("failed to fetch second row for SQL: {sql}"))?
        .is_some()
    {
        bail!("query returned more than one row for SQL: {sql}");
    }
    map(&row).map(Some)
}

fn sqlite_query_all<T>(
    conn: &Connection,
    sql: &str,
    params: &[DbParam<'_>],
    map: &mut impl FnMut(&DbRow) -> anyhow::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let sql = sqlite_placeholders(sql);
    let values = sqlite_params(params);
    let mut stmt = conn
        .prepare(sql.as_ref())
        .with_context(|| format!("failed to prepare SQL: {sql}"))?;
    let mut rows = stmt
        .query(params_from_iter(values.iter()))
        .with_context(|| format!("failed to query SQL: {sql}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .with_context(|| format!("failed to fetch row for SQL: {sql}"))?
    {
        let row = sqlite_row(row)?;
        out.push(map(&row)?);
    }
    Ok(out)
}

fn sqlite_row(row: &rusqlite::Row<'_>) -> anyhow::Result<DbRow> {
    let mut values = Vec::new();
    for index in 0..row.as_ref().column_count() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => DbValue::Null,
            ValueRef::Integer(value) => DbValue::Integer(value),
            ValueRef::Real(value) => DbValue::Real(value),
            ValueRef::Text(value) => DbValue::Text(
                std::str::from_utf8(value)
                    .context("sqlite text column is not valid utf-8")?
                    .to_string(),
            ),
            ValueRef::Blob(_) => bail!("blob columns are not supported by calibration queries"),
        };
        values.push(value);
    }
    Ok(DbRow { values })
}

fn sqlite_params(params: &[DbParam<'_>]) -> Vec<Value> {
    params
        .iter()
        .map(|param| match param {
            DbParam::Null => Value::Null,
            DbParam::Integer(value) => Value::Integer(*value),
            DbParam::Real(value) => Value::Real(*value),
            DbParam::Text(value) => Value::Text((*value).to_string()),
            DbParam::Bool(value) => Value::Integer(i64::from(*value)),
        })
        .collect()
}

fn sqlite_placeholders(sql: &str) -> std::borrow::Cow<'_, str> {
    if !sql.as_bytes().contains(&b'$') {
        return std::borrow::Cow::Borrowed(sql);
    }

    let mut rewritten = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' || !chars.peek().is_some_and(char::is_ascii_digit) {
            rewritten.push(ch);
            continue;
        }

        rewritten.push('?');
        while let Some(next) = chars.peek().copied() {
            if !next.is_ascii_digit() {
                break;
            }
            rewritten.push(next);
            chars.next();
        }
    }
    std::borrow::Cow::Owned(rewritten)
}

struct Migration {
    version: i64,
    name: String,
    sql: String,
}

fn load_migrations(backend: BackendKind) -> anyhow::Result<Vec<Migration>> {
    let migrations = match backend {
        BackendKind::Sqlite => MIGRATIONS,
        #[cfg(feature = "postgres")]
        BackendKind::Postgres => POSTGRES_MIGRATIONS,
    };

    migrations
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
