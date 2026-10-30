//! Read-only access to the MacPorts SQLite registry; never initializes or migrates it.
mod version;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{collections::HashSet, path::Path, time::Duration};

pub const DEFAULT_PATH: &str = "/opt/local/var/macports/registry/registry.db";
#[derive(Debug)]
pub struct Entry {
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
    let explicit = path.is_some();
    let path = path.unwrap_or_else(|| Path::new(DEFAULT_PATH));
    if !explicit && !path.try_exists()? {
        return Ok(Vec::new());
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("cannot open registry {} read-only", path.display()))?;
    connection.create_collation("VERSION", version::compare)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    read_entries(&connection)
        .with_context(|| format!("cannot read MacPorts registry {}", path.display()))
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
    ]
    .map(|column| {
        if columns.contains(column) {
            format!("COALESCE(CAST({column} AS TEXT), '')")
        } else {
            "''".into()
        }
    })
    .join(", ");
    let sql = format!(
        "SELECT name, version, revision, COALESCE(variants, ''), state, {optional} FROM ports WHERE state IN ('installed', 'imaged')"
    );
    let mut query = connection.prepare(&sql)?;
    let mut entries = query
        .query_map([], |row| {
            Ok(Entry {
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
