//! SQLite access through sqlx, behind a synchronous API. Statements are built
//! with sea-query; pragmas, the schema and transaction control are plain SQL.
use super::version;
use anyhow::{Context, Result};
use sea_query::{
    DeleteStatement, InsertStatement, SelectStatement, SqliteQueryBuilder, UpdateStatement,
};
use sea_query_binder::{SqlxBinder, SqlxValues};
use sqlx::{
    ConnectOptions, Connection as _, Executor, Row as _, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteRow, SqliteSynchronous},
};
use std::{future::Future, path::Path, time::Duration};

/// Table and column names of the MacPorts registry schema.
pub mod schema {
    use sea_query::Iden;

    #[derive(Iden, Clone, Copy)]
    pub enum Ports {
        Table,
        Id,
        Name,
        Portfile,
        Location,
        Epoch,
        Version,
        Revision,
        Variants,
        RequestedVariants,
        State,
        Date,
        Installtype,
        Archs,
        Requested,
        OsPlatform,
        OsMajor,
        CxxStdlib,
        CxxStdlibOverridden,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Files {
        Table,
        Id,
        Path,
        ActualPath,
        Active,
        Binary,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Dependencies {
        Table,
        Id,
        Name,
        Variants,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Metadata {
        Table,
        Key,
        Value,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Snapshots {
        Table,
        Id,
        CreatedAt,
        Note,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum SnapshotPorts {
        Table,
        Id,
        SnapshotsId,
        PortName,
        Requested,
        State,
        Variants,
        RequestedVariants,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum SnapshotFiles {
        Table,
        Id,
        Path,
    }
}

const SQLITE_DBCONFIG_DQS_DML: std::ffi::c_int = 1013;
const SQLITE_DBCONFIG_DQS_DDL: std::ffi::c_int = 1014;

/// How to open a database.
pub enum Mode {
    ReadOnly,
    /// An existing database for writing; the file itself must not be a symlink.
    ReadWrite,
    /// Create the database if it does not exist.
    Create,
}

/// One connection with the runtime that drives it.
pub struct Database {
    connection: Option<SqliteConnection>,
    runtime: Option<tokio::runtime::Runtime>,
}

/// Drive a future to completion, also from inside another Tokio runtime.
fn block<F: Future + Send>(runtime: &tokio::runtime::Runtime, future: F) -> F::Output
where
    F::Output: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|scope| {
            scope
                .spawn(|| runtime.block_on(future))
                .join()
                .expect("database worker panicked")
        })
    } else {
        runtime.block_on(future)
    }
}

impl Database {
    pub fn open(path: &Path, mode: Mode) -> Result<Self> {
        if matches!(mode, Mode::ReadWrite) {
            let metadata = std::fs::symlink_metadata(path)
                .with_context(|| format!("cannot open registry {}", path.display()))?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "registry {} must not be a symbolic link",
                path.display()
            );
        }
        let mut options = SqliteConnectOptions::new()
            .filename(path)
            .collation("VERSION", version::compare)
            .busy_timeout(Duration::from_secs(2))
            .foreign_keys(false)
            .disable_statement_logging();
        options = match mode {
            Mode::ReadOnly => options.read_only(true),
            Mode::ReadWrite => options
                .foreign_keys(true)
                .synchronous(SqliteSynchronous::Full)
                .pragma("fullfsync", "ON"),
            Mode::Create => options
                .create_if_missing(true)
                .foreign_keys(true)
                .synchronous(SqliteSynchronous::Full)
                .pragma("fullfsync", "ON"),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let mut connection = block(&runtime, options.connect())
            .with_context(|| format!("cannot open registry {}", path.display()))?;
        // Unknown double-quoted identifiers must be errors, not string
        // literals: statements quote every column name.
        block(&runtime, async {
            let mut handle = connection.lock_handle().await?;
            let database = handle.as_raw_handle().as_ptr();
            for option in [SQLITE_DBCONFIG_DQS_DML, SQLITE_DBCONFIG_DQS_DDL] {
                // SAFETY: the handle is locked and open; the option takes an
                // int and an optional out-pointer.
                let status = unsafe {
                    libsqlite3_sys::sqlite3_db_config(
                        database,
                        option,
                        0 as std::ffi::c_int,
                        std::ptr::null_mut::<std::ffi::c_int>(),
                    )
                };
                anyhow::ensure!(status == 0, "cannot configure SQLite ({status})");
            }
            anyhow::Ok(())
        })?;
        Ok(Self {
            connection: Some(connection),
            runtime: Some(runtime),
        })
    }

    /// Run several plain SQL statements.
    pub fn batch(&mut self, sql: &str) -> Result<()> {
        let runtime = self.runtime.as_ref().expect("open runtime");
        let connection = self.connection.as_mut().expect("open connection");
        block(
            runtime,
            async move { connection.execute(sql).await.map(|_| ()) },
        )?;
        Ok(())
    }

    /// Rows of a plain SQL query without parameters.
    pub fn raw_rows(&mut self, sql: &str) -> Result<Vec<SqliteRow>> {
        let runtime = self.runtime.as_ref().expect("open runtime");
        let connection = self.connection.as_mut().expect("open connection");
        Ok(block(
            runtime,
            async move { connection.fetch_all(sql).await },
        )?)
    }

    fn run_rows(&mut self, sql: String, values: SqlxValues) -> Result<Vec<SqliteRow>> {
        let runtime = self.runtime.as_ref().expect("open runtime");
        let connection = self.connection.as_mut().expect("open connection");
        Ok(block(runtime, async move {
            sqlx::query_with(&sql, values)
                .fetch_all(&mut *connection)
                .await
        })?)
    }

    fn run_execute(&mut self, sql: String, values: SqlxValues) -> Result<(u64, i64)> {
        let runtime = self.runtime.as_ref().expect("open runtime");
        let connection = self.connection.as_mut().expect("open connection");
        let result = block(runtime, async move {
            sqlx::query_with(&sql, values)
                .execute(&mut *connection)
                .await
        })?;
        Ok((result.rows_affected(), result.last_insert_rowid()))
    }

    pub fn select(&mut self, statement: &SelectStatement) -> Result<Vec<SqliteRow>> {
        let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
        self.run_rows(sql, values)
    }

    pub fn select_one(&mut self, statement: &SelectStatement) -> Result<Option<SqliteRow>> {
        Ok(self.select(statement)?.into_iter().next())
    }

    /// Insert one row and return its rowid.
    pub fn insert(&mut self, statement: &InsertStatement) -> Result<i64> {
        let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
        Ok(self.run_execute(sql, values)?.1)
    }

    /// Run an update and return the number of changed rows.
    pub fn update(&mut self, statement: &UpdateStatement) -> Result<u64> {
        let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
        Ok(self.run_execute(sql, values)?.0)
    }

    pub fn delete(&mut self, statement: &DeleteStatement) -> Result<u64> {
        let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
        Ok(self.run_execute(sql, values)?.0)
    }

    /// Column names of a table.
    pub fn columns(&mut self, table: &str) -> Result<Vec<String>> {
        let rows = self.raw_rows(&format!("PRAGMA table_info({table})"))?;
        rows.iter()
            .map(|row| row.try_get::<String, _>(1).map_err(Into::into))
            .collect()
    }

    /// Run `body` in a savepoint, rolled back if it fails.
    pub fn savepoint<T>(&mut self, body: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.batch("SAVEPOINT port_rs")?;
        match body(self) {
            Ok(value) => {
                self.batch("RELEASE port_rs")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.batch("ROLLBACK TO port_rs; RELEASE port_rs");
                Err(error)
            }
        }
    }

    pub fn close(mut self) -> Result<()> {
        if let Some(connection) = self.connection.take() {
            block(
                self.runtime.as_ref().expect("open runtime"),
                connection.close(),
            )?;
        }
        Ok(())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            if let Some(runtime) = &self.runtime {
                let _ = block(runtime, connection.close());
            }
        }
        // A runtime may not be dropped inside another runtime's async context;
        // shutting it down in the background is allowed anywhere.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// A rusqlite-style connection for raw SQL, used to inspect or prepare
/// registries outside the typed API (fixtures and diagnostics).
pub mod sqlite {
    use super::{Database, Mode, block};
    use anyhow::{Context, Result};
    use sqlx::{Row as _, sqlite::SqliteRow};
    use std::path::Path;

    /// A value bound to a `?` or `?N` parameter.
    #[derive(Clone, Debug)]
    pub enum Value {
        Null,
        Integer(i64),
        Real(f64),
        Text(String),
        Blob(Vec<u8>),
    }

    macro_rules! integer {
        ($($type:ty),*) => {$(
            impl From<$type> for Value {
                fn from(value: $type) -> Self {
                    Value::Integer(value as i64)
                }
            }
        )*};
    }
    integer!(i8, i16, i32, i64, u8, u16, u32, isize, usize);
    impl From<bool> for Value {
        fn from(value: bool) -> Self {
            Value::Integer(value as i64)
        }
    }
    impl From<f64> for Value {
        fn from(value: f64) -> Self {
            Value::Real(value)
        }
    }
    impl From<&str> for Value {
        fn from(value: &str) -> Self {
            Value::Text(value.into())
        }
    }
    impl From<String> for Value {
        fn from(value: String) -> Self {
            Value::Text(value)
        }
    }
    impl From<&&str> for Value {
        fn from(value: &&str) -> Self {
            Value::Text((*value).into())
        }
    }
    impl From<&String> for Value {
        fn from(value: &String) -> Self {
            Value::Text(value.clone())
        }
    }
    impl From<Vec<u8>> for Value {
        fn from(value: Vec<u8>) -> Self {
            Value::Blob(value)
        }
    }
    impl<T: Into<Value>> From<Option<T>> for Value {
        fn from(value: Option<T>) -> Self {
            value.map(Into::into).unwrap_or(Value::Null)
        }
    }
    macro_rules! by_reference {
        ($($type:ty),*) => {$(
            impl From<&$type> for Value {
                fn from(value: &$type) -> Self {
                    (*value).into()
                }
            }
        )*};
    }
    by_reference!(i8, i16, i32, i64, u8, u16, u32, isize, usize, bool, f64);

    /// Statement parameters: `()`, an array, or `params![...]`.
    pub trait Params {
        fn values(self) -> Vec<Value>;
    }
    impl Params for () {
        fn values(self) -> Vec<Value> {
            Vec::new()
        }
    }
    impl Params for Vec<Value> {
        fn values(self) -> Vec<Value> {
            self
        }
    }
    impl<T: Into<Value>, const N: usize> Params for [T; N] {
        fn values(self) -> Vec<Value> {
            self.into_iter().map(Into::into).collect()
        }
    }

    /// One result row.
    pub struct Row(SqliteRow);
    impl Row {
        pub fn get<I, T>(&self, index: I) -> Result<T>
        where
            I: sqlx::ColumnIndex<SqliteRow>,
            T: for<'r> sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
        {
            Ok(self.0.try_get(index)?)
        }
    }

    pub struct Connection(Database);
    impl Connection {
        /// Open or create a database for reading and writing.
        pub fn open(path: impl AsRef<Path>) -> Result<Self> {
            Ok(Self(Database::open(path.as_ref(), Mode::Create)?))
        }
        pub fn execute_batch(&mut self, sql: &str) -> Result<()> {
            self.0.batch(sql)
        }
        /// Replace a collation on this connection.
        pub fn create_collation(
            &mut self,
            name: &str,
            compare: impl Fn(&str, &str) -> std::cmp::Ordering + Send + Sync + 'static,
        ) -> Result<()> {
            let database = &mut self.0;
            let runtime = database.runtime.as_ref().expect("open runtime");
            let connection = database.connection.as_mut().expect("open connection");
            let name = name.to_string();
            block(runtime, async move {
                let mut handle = connection.lock_handle().await?;
                handle.create_collation(&name, compare)?;
                anyhow::Ok(())
            })
        }
        /// Compile a statement without running it.
        pub fn prepare(&mut self, sql: &str) -> Result<()> {
            use sqlx::Executor as _;
            let database = &mut self.0;
            let runtime = database.runtime.as_ref().expect("open runtime");
            let connection = database.connection.as_mut().expect("open connection");
            block(
                runtime,
                async move { connection.prepare(sql).await.map(|_| ()) },
            )?;
            Ok(())
        }
        /// Run one statement and return the number of changed rows.
        pub fn execute(&mut self, sql: &str, params: impl Params) -> Result<usize> {
            let values = params.values();
            let database = &mut self.0;
            let runtime = database.runtime.as_ref().expect("open runtime");
            let connection = database.connection.as_mut().expect("open connection");
            let result = block(runtime, async move {
                let mut query = sqlx::query(sql);
                for value in values {
                    query = match value {
                        Value::Null => query.bind(None::<i64>),
                        Value::Integer(value) => query.bind(value),
                        Value::Real(value) => query.bind(value),
                        Value::Text(value) => query.bind(value),
                        Value::Blob(value) => query.bind(value),
                    };
                }
                query.execute(&mut *connection).await
            })?;
            Ok(result.rows_affected() as usize)
        }
        /// Map the first row of a query; an error when there is none.
        pub fn query_row<T>(
            &mut self,
            sql: &str,
            params: impl Params,
            map: impl FnOnce(&Row) -> Result<T>,
        ) -> Result<T> {
            let rows = self.query(sql, params)?;
            let row = rows.into_iter().next().context("query returned no rows")?;
            map(&row)
        }
        /// All rows of a query.
        pub fn query(&mut self, sql: &str, params: impl Params) -> Result<Vec<Row>> {
            let values = params.values();
            let database = &mut self.0;
            let runtime = database.runtime.as_ref().expect("open runtime");
            let connection = database.connection.as_mut().expect("open connection");
            let rows = block(runtime, async move {
                let mut query = sqlx::query(sql);
                for value in values {
                    query = match value {
                        Value::Null => query.bind(None::<i64>),
                        Value::Integer(value) => query.bind(value),
                        Value::Real(value) => query.bind(value),
                        Value::Text(value) => query.bind(value),
                        Value::Blob(value) => query.bind(value),
                    };
                }
                query.fetch_all(&mut *connection).await
            })?;
            Ok(rows.into_iter().map(Row).collect())
        }
    }

    /// Statement parameters of mixed types, like rusqlite's `params!`.
    #[macro_export]
    macro_rules! params {
        () => { ::std::vec::Vec::<$crate::sqlite::Value>::new() };
        ($($value:expr),+ $(,)?) => {
            ::std::vec![$($crate::sqlite::Value::from($value)),+]
        };
    }
}
