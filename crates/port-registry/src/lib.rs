//! Native MacPorts registry access. Readers never initialize or migrate databases.
pub use port_core::{index, order, query, spec};
mod db;
pub mod dependents;
pub mod expression;
mod leaves;
pub mod path;
mod selection;
pub mod snapshot;
pub mod version;
pub mod write;
use anyhow::{Context, Result, ensure};
pub use db::sqlite;
use db::{
    Database, Mode,
    schema::{Dependencies, Files, Ports},
};
use sea_query::{Alias, Expr, Func, Order, Query};
pub use selection::specifications;
use sqlx::Row as _;
use std::{cell::RefCell, collections::HashSet, path::Path};

pub const DEFAULT_PATH: &str = "/opt/local/var/macports/registry/registry.db";
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub epoch: String,
    pub name: String,
    pub version: String,
    pub revision: i64,
    pub variants: String,
    pub active: bool,
    pub requested: bool,
    pub requested_variants: String,
    pub platform: String,
    pub os_major: String,
    pub archs: String,
    pub date: String,
}
impl Entry {
    pub fn full_version(&self) -> String {
        format!("{}_{}{}", self.version, self.revision, self.variants)
    }
}

pub fn read(path: Option<&Path>) -> Result<Vec<Entry>> {
    match Registry::open(path)? {
        Some(registry) => registry.entries(),
        None => Ok(Vec::new()),
    }
}

/// Holds a read transaction so related entry and file queries see one snapshot.
pub struct Registry {
    database: RefCell<Database>,
}
impl Registry {
    pub fn open(path: Option<&Path>) -> Result<Option<Self>> {
        let explicit = path.is_some();
        let path = path.unwrap_or_else(|| Path::new(DEFAULT_PATH));
        if !explicit && !path.try_exists()? {
            return Ok(None);
        }
        let mut database = Database::open(path, Mode::ReadOnly)
            .with_context(|| format!("cannot open registry {} read-only", path.display()))?;
        database.batch("BEGIN")?;
        Ok(Some(Self {
            database: RefCell::new(database),
        }))
    }
    pub(crate) fn database(&self) -> std::cell::RefMut<'_, Database> {
        self.database.borrow_mut()
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        read_entries(&mut self.database()).context("cannot read MacPorts registry entries")
    }
    /// Match native registry entry search, including schema collations and both
    /// active and inactive images. Epoch is not part of entry_exists identity.
    pub fn contains_image(
        &self,
        name: &str,
        version: &str,
        revision: &str,
        variants: &str,
    ) -> Result<bool> {
        let statement = Query::select()
            .expr(Expr::val(1))
            .from(Ports::Table)
            .and_where(Expr::col(Ports::Name).eq(name))
            .and_where(Expr::col(Ports::Version).eq(version))
            .and_where(Expr::col(Ports::Revision).eq(revision))
            .and_where(Expr::col(Ports::Variants).eq(variants))
            .limit(1)
            .to_owned();
        Ok(self.database().select_one(&statement)?.is_some())
    }
    pub fn contains_name(&self, name: &str) -> Result<bool> {
        let statement = Query::select()
            .expr(Expr::val(1))
            .from(Ports::Table)
            .and_where(Expr::col(Ports::Name).eq(name))
            .limit(1)
            .to_owned();
        Ok(self.database().select_one(&statement)?.is_some())
    }
    /// Look up active file-map ownership using the filesystem's case rules.
    pub fn owner(&self, path: &str, case_sensitive: bool) -> Result<Option<String>> {
        let matches = if case_sensitive {
            Expr::col((Files::Table, Files::ActualPath)).eq(path)
        } else {
            Expr::cust_with_values("\"files\".\"actual_path\" = ? COLLATE NOCASE", [path])
        };
        let statement = Query::select()
            .column((Ports::Table, Ports::Name))
            .from(Files::Table)
            .inner_join(
                Ports::Table,
                Expr::col((Ports::Table, Ports::Id)).equals((Files::Table, Files::Id)),
            )
            .and_where(matches)
            .and_where(Expr::col((Files::Table, Files::Active)).ne(0))
            .limit(1)
            .to_owned();
        self.database()
            .select_one(&statement)
            .with_context(|| format!("cannot look up registry owner for {path}"))?
            .map(|row| row.try_get(0).map_err(Into::into))
            .transpose()
    }
    /// Active files flagged as Mach-O or ar binaries, for rev-upgrade.
    pub fn active_binaries(&self) -> Result<Vec<String>> {
        let statement = Query::select()
            .expr(Func::coalesce([
                Expr::col(Files::ActualPath).into(),
                Expr::col(Files::Path).into(),
            ]))
            .from(Files::Table)
            .and_where(Expr::col(Files::Active).eq(1))
            .and_where(Expr::col(Files::Binary).eq(1))
            .to_owned();
        self.database()
            .select(&statement)
            .context("cannot read registry binaries")?
            .iter()
            .map(|row| row.try_get(0).map_err(Into::into))
            .collect()
    }
    /// Installed ports built against the other C++ standard library.
    pub fn cxx_stdlib_mismatches(&self, wrong: &str) -> Result<Vec<i64>> {
        let statement = Query::select()
            .column(Ports::Id)
            .from(Ports::Table)
            .and_where(Expr::col(Ports::State).eq("installed"))
            .and_where(Expr::col(Ports::CxxStdlibOverridden).eq(0))
            .and_where(Expr::col(Ports::CxxStdlib).eq(wrong))
            .to_owned();
        // Older registries have no C++ standard library columns.
        let Ok(rows) = self.database().select(&statement) else {
            return Ok(Vec::new());
        };
        rows.iter()
            .map(|row| row.try_get(0).map_err(Into::into))
            .collect()
    }
    pub fn dependent_edges(&self) -> Result<Vec<(String, i64)>> {
        let statement = Query::select()
            .column((Dependencies::Table, Dependencies::Name))
            .column((Dependencies::Table, Dependencies::Id))
            .from(Dependencies::Table)
            .inner_join(
                Ports::Table,
                Expr::col((Ports::Table, Ports::Id))
                    .equals((Dependencies::Table, Dependencies::Id)),
            )
            .and_where(Expr::col((Ports::Table, Ports::State)).is_in(["installed", "imaged"]))
            .to_owned();
        self.database()
            .select(&statement)
            .context("cannot read registry dependencies")?
            .iter()
            .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
            .collect()
    }
    /// Return the recorded image location without inspecting or unpacking it.
    pub fn location(&self, entry: &Entry) -> Result<String> {
        let statement = Query::select()
            .column(Ports::Location)
            .from(Ports::Table)
            .and_where(Expr::col(Ports::Id).eq(entry.id))
            .to_owned();
        let context = || {
            format!(
                "cannot read image location for {} @{}",
                entry.name,
                entry.full_version()
            )
        };
        let row = self
            .database()
            .select_one(&statement)
            .with_context(context)?
            .with_context(context)?;
        row.try_get(0).with_context(context)
    }
    pub fn files(&self, entry: &Entry) -> Result<Vec<String>> {
        let column = if entry.active {
            Files::ActualPath
        } else {
            Files::Path
        };
        let mut statement = Query::select();
        statement
            .column(column)
            .from(Files::Table)
            .and_where(Expr::col(Files::Id).eq(entry.id))
            .order_by(column, Order::Asc);
        if entry.active {
            statement.and_where(Expr::col(Files::Active).ne(0));
        }
        let files: Vec<Option<String>> = self
            .database()
            .select(&statement)
            .with_context(|| format!("cannot read files for {}", entry.name))?
            .iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()?;
        Ok(files.into_iter().flatten().collect())
    }
}

/// Entries of the ports table, tolerating registries without optional columns.
fn read_entries(database: &mut Database) -> Result<Vec<Entry>> {
    let columns: HashSet<String> = database.columns("ports")?.into_iter().collect();
    for required in ["name", "version", "revision", "variants", "state"] {
        ensure!(
            columns.contains(required),
            "unsupported registry schema: missing ports.{required}"
        );
    }
    let mut statement = Query::select();
    statement
        .column(Ports::Name)
        .column(Ports::Version)
        .column(Ports::Revision)
        .expr(Func::coalesce([
            Expr::col(Ports::Variants).into(),
            Expr::val("").into(),
        ]))
        .column(Ports::State);
    for column in [
        "requested",
        "requested_variants",
        "os_platform",
        "os_major",
        "archs",
        "date",
        "epoch",
    ] {
        if columns.contains(column) {
            statement.expr(Expr::cust(format!(
                "COALESCE(CAST(\"{column}\" AS TEXT), '')"
            )));
        } else {
            statement.expr(Expr::val(""));
        }
    }
    statement
        .expr(Expr::col(Alias::new(if columns.contains("id") {
            "id"
        } else {
            "rowid"
        })))
        .from(Ports::Table)
        .and_where(Expr::col(Ports::State).is_in(["installed", "imaged"]));
    let mut entries = database
        .select(&statement)?
        .iter()
        .map(|row| {
            Ok(Entry {
                id: row.try_get(12)?,
                epoch: row.try_get(11)?,
                name: row.try_get(0)?,
                version: row.try_get(1)?,
                revision: row.try_get(2)?,
                variants: row.try_get(3)?,
                active: row.try_get::<String, _>(4)? == "installed",
                requested: row.try_get::<String, _>(5)? == "1",
                requested_variants: row.try_get(6)?,
                platform: row.try_get(7)?,
                os_major: row.try_get(8)?,
                archs: row.try_get(9)?,
                date: row.try_get(10)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|a, b| {
        let names = if a.name.eq_ignore_ascii_case(&b.name) {
            std::cmp::Ordering::Equal
        } else {
            crate::order::dictionary(&a.name, &b.name)
        };
        names
            .then_with(|| version::compare(&a.version, &b.version))
            .then_with(|| a.revision.cmp(&b.revision))
            .then_with(|| {
                a.variants
                    .to_ascii_lowercase()
                    .cmp(&b.variants.to_ascii_lowercase())
            })
    });
    Ok(entries)
}

pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    version::compare(a, b)
}
