//! Native MacPorts registry access. Readers never initialize or migrate databases.
pub mod dependents;
pub mod expression;
mod leaves;
pub mod path;
mod selection;
pub(crate) mod version;
pub mod write;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
pub use selection::specifications;
use std::{collections::HashSet, path::Path, time::Duration};

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
    connection: Connection,
}
impl Registry {
    pub fn open(path: Option<&Path>) -> Result<Option<Self>> {
        let explicit = path.is_some();
        let path = path.unwrap_or_else(|| Path::new(DEFAULT_PATH));
        if !explicit && !path.try_exists()? {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("cannot open registry {} read-only", path.display()))?;
        connection.create_collation("VERSION", version::compare)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("BEGIN")?;
        Ok(Some(Self { connection }))
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        read_entries(&self.connection).context("cannot read MacPorts registry entries")
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
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ports WHERE name=?1 AND version=?2 AND revision=?3 AND variants=?4)",
            rusqlite::params![name, version, revision, variants], |row| row.get(0))?)
    }
    pub fn contains_name(&self, name: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ports WHERE name=?1)",
            [name],
            |row| row.get(0),
        )?)
    }
    /// Look up active file-map ownership using the filesystem's case rules.
    pub fn owner(&self, path: &str, case_sensitive: bool) -> Result<Option<String>> {
        let sql = if case_sensitive {
            "SELECT ports.name FROM files JOIN ports ON ports.id=files.id WHERE files.actual_path=?1 AND files.active LIMIT 1"
        } else {
            "SELECT ports.name FROM files JOIN ports ON ports.id=files.id WHERE files.actual_path=?1 COLLATE NOCASE AND files.active LIMIT 1"
        };
        self.connection
            .query_row(sql, [path], |row| row.get(0))
            .optional()
            .with_context(|| format!("cannot look up registry owner for {path}"))
    }
    pub fn dependent_edges(&self) -> Result<Vec<(String, i64)>> {
        let mut statement = self.connection.prepare(
            "SELECT dependencies.name, dependencies.id FROM dependencies JOIN ports ON ports.id=dependencies.id WHERE ports.state IN ('installed', 'imaged')"
        ).context("cannot read registry dependencies")?;
        Ok(statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Return the recorded image location without inspecting or unpacking it.
    pub fn location(&self, entry: &Entry) -> Result<String> {
        self.connection
            .query_row(
                "SELECT location FROM ports WHERE id=?1",
                [entry.id],
                |row| row.get(0),
            )
            .with_context(|| {
                format!(
                    "cannot read image location for {} @{}",
                    entry.name,
                    entry.full_version()
                )
            })
    }
    pub fn files(&self, entry: &Entry) -> Result<Vec<String>> {
        let sql = if entry.active {
            "SELECT actual_path FROM files WHERE id=?1 AND active ORDER BY actual_path"
        } else {
            "SELECT path FROM files WHERE id=?1 ORDER BY path"
        };
        let mut statement = self
            .connection
            .prepare(sql)
            .with_context(|| format!("cannot read files for {}", entry.name))?;
        let files = statement
            .query_map([entry.id], |row| row.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(files.into_iter().flatten().collect())
    }
}
fn read_entries(connection: &Connection) -> Result<Vec<Entry>> {
    let columns = connection
        .prepare("PRAGMA table_info(ports)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    for required in ["name", "version", "revision", "variants", "state"] {
        ensure!(
            columns.contains(required),
            "unsupported registry schema: missing ports.{required}"
        );
    }
    let optional = [
        "requested",
        "requested_variants",
        "os_platform",
        "os_major",
        "archs",
        "date",
        "epoch",
    ]
    .map(|column| {
        if columns.contains(column) {
            format!("COALESCE(CAST({column} AS TEXT), '')")
        } else {
            "''".into()
        }
    })
    .join(", ");
    let identity = if columns.contains("id") {
        "id"
    } else {
        "rowid"
    };
    let sql = format!(
        "SELECT name, version, revision, COALESCE(variants, ''), state, {optional}, {identity} FROM ports WHERE state IN ('installed', 'imaged')"
    );
    let mut query = connection.prepare(&sql)?;
    let mut entries = query
        .query_map([], |row| {
            Ok(Entry {
                id: row.get(12)?,
                epoch: row.get(11)?,
                name: row.get(0)?,
                version: row.get(1)?,
                revision: row.get(2)?,
                variants: row.get(3)?,
                active: row.get::<_, String>(4)? == "installed",
                requested: row.get::<_, String>(5)? == "1",
                requested_variants: row.get(6)?,
                platform: row.get(7)?,
                os_major: row.get(8)?,
                archs: row.get(9)?,
                date: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
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

pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    version::compare(a, b)
}
