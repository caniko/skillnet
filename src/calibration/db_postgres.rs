use std::{
    cell::{Cell, RefCell},
    error::Error,
};

use anyhow::{bail, Context};
use postgres::{
    types::{private::BytesMut, to_sql_checked, IsNull, Json, ToSql, Type},
    Client, NoTls, Row, Transaction,
};
use serde_json::Value as JsonValue;

use super::{DbParam, DbRow, DbValue};

pub struct PostgresBackend {
    client: RefCell<Client>,
    last_insert_id: Cell<Option<i64>>,
}

pub struct PostgresTransaction<'db> {
    tx: Transaction<'db>,
    last_insert_id: &'db Cell<Option<i64>>,
}

enum ParamValue {
    Null(PgNull),
    Integer(PgInteger),
    Real(f64),
    Text(String),
    Json(Json<JsonValue>),
    Bool(bool),
}

#[derive(Debug)]
struct PgNull;

#[derive(Debug)]
struct PgInteger(i64);

impl PostgresBackend {
    pub fn connect(url: &str) -> anyhow::Result<Self> {
        let client = Client::connect(url, NoTls).context("failed to connect to postgres")?;
        Ok(Self {
            client: RefCell::new(client),
            last_insert_id: Cell::new(None),
        })
    }

    pub fn execute(&self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<usize> {
        execute_client(
            &mut *self.client.borrow_mut(),
            &self.last_insert_id,
            sql,
            params,
        )
    }

    pub fn execute_returning_id(&self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<i64> {
        let sql = format!("{} RETURNING id", sql.trim_end().trim_end_matches(';'));
        let values = pg_params(params);
        let refs = pg_param_refs(&values);
        let row = self
            .client
            .borrow_mut()
            .query_one(&sql, &refs)
            .with_context(|| format!("failed to execute postgres SQL: {sql}"))?;
        let id = row_i64(&row, 0)?;
        self.last_insert_id.set(Some(id));
        Ok(id)
    }

    pub fn execute_batch(&self, sql: &str) -> anyhow::Result<()> {
        self.client
            .borrow_mut()
            .batch_execute(sql)
            .context("failed to execute postgres SQL batch")
    }

    pub fn query_optional<T>(
        &self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Option<T>> {
        query_optional_client(&mut *self.client.borrow_mut(), sql, params, map)
    }

    pub fn query_all<T>(
        &self,
        sql: &str,
        params: &[DbParam<'_>],
        map: &mut impl FnMut(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Vec<T>> {
        query_all_client(&mut *self.client.borrow_mut(), sql, params, map)
    }

    pub fn transaction<T>(
        &mut self,
        f: impl FnOnce(PostgresTransaction<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let tx = self
            .client
            .get_mut()
            .transaction()
            .context("failed to start postgres transaction")?;
        f(PostgresTransaction {
            tx,
            last_insert_id: &self.last_insert_id,
        })
    }
}

impl PostgresTransaction<'_> {
    pub fn execute(&mut self, sql: &str, params: &[DbParam<'_>]) -> anyhow::Result<usize> {
        execute_client(&mut self.tx, self.last_insert_id, sql, params)
    }

    pub fn execute_batch(&mut self, sql: &str) -> anyhow::Result<()> {
        self.tx
            .batch_execute(sql)
            .context("failed to execute postgres SQL batch")
    }

    pub fn query_optional<T>(
        &mut self,
        sql: &str,
        params: &[DbParam<'_>],
        map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Option<T>> {
        query_optional_client(&mut self.tx, sql, params, map)
    }

    pub fn query_all<T>(
        &mut self,
        sql: &str,
        params: &[DbParam<'_>],
        map: &mut impl FnMut(&DbRow) -> anyhow::Result<T>,
    ) -> anyhow::Result<Vec<T>> {
        query_all_client(&mut self.tx, sql, params, map)
    }

    pub fn commit(self) -> anyhow::Result<()> {
        self.tx
            .commit()
            .context("failed to commit postgres transaction")
    }
}

trait PgClient {
    fn execute_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error>;

    fn query_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, postgres::Error>;

    fn query_opt_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, postgres::Error>;
}

impl PgClient for Client {
    fn execute_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        self.execute(sql, params)
    }

    fn query_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, postgres::Error> {
        self.query(sql, params)
    }

    fn query_opt_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, postgres::Error> {
        self.query_opt(sql, params)
    }
}

impl PgClient for Transaction<'_> {
    fn execute_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error> {
        self.execute(sql, params)
    }

    fn query_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, postgres::Error> {
        self.query(sql, params)
    }

    fn query_opt_pg(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, postgres::Error> {
        self.query_opt(sql, params)
    }
}

fn execute_client(
    client: &mut impl PgClient,
    last_insert_id: &Cell<Option<i64>>,
    sql: &str,
    params: &[DbParam<'_>],
) -> anyhow::Result<usize> {
    let values = pg_params(params);
    let refs = pg_param_refs(&values);
    let changed = client
        .execute_pg(sql, &refs)
        .with_context(|| format!("failed to execute postgres SQL: {sql}"))?;
    if sql.trim_start().to_ascii_uppercase().starts_with("INSERT ") {
        if let Ok(Some(row)) = client.query_opt_pg("SELECT lastval()", &[]) {
            if let Ok(id) = row.try_get::<_, i64>(0) {
                last_insert_id.set(Some(id));
            }
        }
    }
    usize::try_from(changed).context("postgres changed-row count does not fit in usize")
}

fn query_optional_client<T>(
    client: &mut impl PgClient,
    sql: &str,
    params: &[DbParam<'_>],
    map: impl FnOnce(&DbRow) -> anyhow::Result<T>,
) -> anyhow::Result<Option<T>> {
    let values = pg_params(params);
    let refs = pg_param_refs(&values);
    let Some(row) = client
        .query_opt_pg(sql, &refs)
        .with_context(|| format!("failed to query postgres SQL: {sql}"))?
    else {
        return Ok(None);
    };
    let row = pg_row(&row)?;
    map(&row).map(Some)
}

fn query_all_client<T>(
    client: &mut impl PgClient,
    sql: &str,
    params: &[DbParam<'_>],
    map: &mut impl FnMut(&DbRow) -> anyhow::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let values = pg_params(params);
    let refs = pg_param_refs(&values);
    let rows = client
        .query_pg(sql, &refs)
        .with_context(|| format!("failed to query postgres SQL: {sql}"))?;
    rows.iter()
        .map(|row| {
            let row = pg_row(row)?;
            map(&row)
        })
        .collect()
}

fn pg_params(params: &[DbParam<'_>]) -> Vec<ParamValue> {
    params
        .iter()
        .map(|param| match param {
            DbParam::Null => ParamValue::Null(PgNull),
            DbParam::Integer(value) => ParamValue::Integer(PgInteger(*value)),
            DbParam::Real(value) => ParamValue::Real(*value),
            DbParam::Text(value) => match serde_json::from_str::<JsonValue>(value) {
                Ok(value) => ParamValue::Json(Json(value)),
                Err(_) => ParamValue::Text((*value).to_string()),
            },
            DbParam::Bool(value) => ParamValue::Bool(*value),
        })
        .collect()
}

fn pg_param_refs(params: &[ParamValue]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|param| match param {
            ParamValue::Null(value) => value as &(dyn ToSql + Sync),
            ParamValue::Integer(value) => value as &(dyn ToSql + Sync),
            ParamValue::Real(value) => value as &(dyn ToSql + Sync),
            ParamValue::Text(value) => value as &(dyn ToSql + Sync),
            ParamValue::Json(value) => value as &(dyn ToSql + Sync),
            ParamValue::Bool(value) => value as &(dyn ToSql + Sync),
        })
        .collect()
}

fn row_i64(row: &Row, index: usize) -> anyhow::Result<i64> {
    if row.columns()[index].type_() == &Type::INT4 {
        Ok(i64::from(row.try_get::<_, i32>(index)?))
    } else {
        Ok(row.try_get::<_, i64>(index)?)
    }
}

fn pg_row(row: &Row) -> anyhow::Result<DbRow> {
    let mut values = Vec::with_capacity(row.len());
    for (index, column) in row.columns().iter().enumerate() {
        let value = if column.type_() == &Type::BOOL {
            row.try_get::<_, Option<bool>>(index)?
                .map_or(DbValue::Null, DbValue::Bool)
        } else if column.type_() == &Type::INT2 {
            row.try_get::<_, Option<i16>>(index)?
                .map_or(DbValue::Null, |value| DbValue::Integer(i64::from(value)))
        } else if column.type_() == &Type::INT4 {
            row.try_get::<_, Option<i32>>(index)?
                .map_or(DbValue::Null, |value| DbValue::Integer(i64::from(value)))
        } else if column.type_() == &Type::INT8 {
            row.try_get::<_, Option<i64>>(index)?
                .map_or(DbValue::Null, DbValue::Integer)
        } else if column.type_() == &Type::FLOAT4 {
            row.try_get::<_, Option<f32>>(index)?
                .map_or(DbValue::Null, |value| DbValue::Real(f64::from(value)))
        } else if column.type_() == &Type::FLOAT8 {
            row.try_get::<_, Option<f64>>(index)?
                .map_or(DbValue::Null, DbValue::Real)
        } else if column.type_() == &Type::JSON || column.type_() == &Type::JSONB {
            row.try_get::<_, Option<Json<JsonValue>>>(index)?
                .map_or(DbValue::Null, |value| DbValue::Text(value.0.to_string()))
        } else if column.type_() == &Type::TEXT
            || column.type_() == &Type::VARCHAR
            || column.type_() == &Type::BPCHAR
            || column.type_() == &Type::NAME
        {
            row.try_get::<_, Option<String>>(index)?
                .map_or(DbValue::Null, DbValue::Text)
        } else {
            bail!(
                "unsupported postgres column type {:?} for column {}",
                column.type_(),
                column.name()
            );
        };
        values.push(value);
    }
    Ok(DbRow { values })
}

impl ToSql for PgNull {
    fn to_sql(
        &self,
        _ty: &Type,
        _out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>>
    where
        Self: Sized,
    {
        Ok(IsNull::Yes)
    }

    fn accepts(_ty: &Type) -> bool
    where
        Self: Sized,
    {
        true
    }

    to_sql_checked!();
}

impl ToSql for PgInteger {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>>
    where
        Self: Sized,
    {
        match *ty {
            Type::INT2 => i16::try_from(self.0)?.to_sql(ty, out),
            Type::INT4 => i32::try_from(self.0)?.to_sql(ty, out),
            _ => self.0.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool
    where
        Self: Sized,
    {
        true
    }

    to_sql_checked!();
}
