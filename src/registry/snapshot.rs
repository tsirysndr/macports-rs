//! Registry snapshots (registry2.0 `registry::snapshot`): the active ports,
//! their requested state and variants, and their files at a point in time.
use super::{Registry, write::WriteTransaction};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: i64,
    /// UTC, `YYYY-MM-DD HH:MM:SS`.
    pub created_at: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotPort {
    pub port_name: String,
    pub requested: i64,
    pub state: String,
    pub variants: String,
    pub requested_variants: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_files: Vec<String>,
}

fn snapshots(connection: &Connection) -> Result<Vec<Snapshot>> {
    let mut statement = connection
        .prepare("SELECT id, created_at, COALESCE(note, '') FROM snapshots ORDER BY id DESC")
        .context("cannot read registry snapshots")?;
    Ok(statement
        .query_map([], |row| {
            Ok(Snapshot {
                id: row.get(0)?,
                created_at: row.get(1)?,
                note: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

impl Registry {
    /// Newest first.
    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        snapshots(&self.connection)
    }
    pub fn snapshot(&self, id: i64) -> Result<Option<Snapshot>> {
        Ok(self
            .snapshots()?
            .into_iter()
            .find(|snapshot| snapshot.id == id))
    }
    /// Ports in a snapshot, ordered by name; files are filled in on request.
    pub fn snapshot_ports(&self, id: i64, include_files: bool) -> Result<Vec<SnapshotPort>> {
        let mut statement = self.connection.prepare(
            "SELECT id, port_name, requested, state, variants, requested_variants FROM snapshot_ports WHERE snapshots_id=?1 ORDER BY port_name",
        )?;
        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    SnapshotPort {
                        port_name: row.get(1)?,
                        requested: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        state: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        variants: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        requested_variants: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                        port_files: Vec::new(),
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut files = self
            .connection
            .prepare("SELECT path FROM snapshot_files WHERE id=?1 ORDER BY path")?;
        let mut ports = Vec::new();
        for (row, mut port) in rows {
            if include_files {
                port.port_files = files
                    .query_map([row], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
            }
            ports.push(port);
        }
        Ok(ports)
    }
    /// The port in a snapshot that owned a file.
    pub fn snapshot_file_owner(&self, path: &str, id: i64) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT snapshot_ports.port_name FROM snapshot_ports JOIN snapshot_files ON snapshot_files.id=snapshot_ports.id WHERE snapshot_files.path=?1 AND snapshot_ports.snapshots_id=?2 LIMIT 1",
                params![path, id],
                |row| row.get(0),
            )
            .optional()?)
    }
}

impl WriteTransaction<'_> {
    /// Record every active port with its files.
    pub fn create_snapshot(&mut self, note: &str) -> Result<i64> {
        let connection = self.connection();
        connection.execute("INSERT INTO snapshots(note) VALUES(?1)", [note])?;
        let snapshot = connection.last_insert_rowid();
        let ports: Vec<(i64, SnapshotPort)> = {
            let mut statement = connection.prepare(
                "SELECT id, name, requested, state, variants, requested_variants FROM ports WHERE state='installed' ORDER BY name",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        SnapshotPort {
                            port_name: row.get(1)?,
                            requested: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            state: row.get(3)?,
                            variants: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                            requested_variants: row
                                .get::<_, Option<String>>(5)?
                                .unwrap_or_default(),
                            port_files: Vec::new(),
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, port) in ports {
            let row = insert_port(connection, snapshot, &port)?;
            connection.execute(
                "INSERT INTO snapshot_files(id, path) SELECT ?1, path FROM files WHERE id=?2",
                params![row, id],
            )?;
        }
        Ok(snapshot)
    }
    pub fn import_snapshot(
        &mut self,
        created_at: &str,
        note: &str,
        ports: &[SnapshotPort],
    ) -> Result<i64> {
        let connection = self.connection();
        connection.execute(
            "INSERT INTO snapshots(created_at, note) VALUES(?1, ?2)",
            params![created_at, note],
        )?;
        let snapshot = connection.last_insert_rowid();
        for port in ports {
            let row = insert_port(connection, snapshot, port)?;
            for path in &port.port_files {
                connection.execute(
                    "INSERT INTO snapshot_files(id, path) VALUES(?1, ?2)",
                    params![row, path],
                )?;
            }
        }
        Ok(snapshot)
    }
    /// Delete a snapshot with its ports and files; false when it does not exist.
    pub fn delete_snapshot(&mut self, id: i64) -> Result<bool> {
        let connection = self.connection();
        connection.execute(
            "DELETE FROM snapshot_files WHERE id IN (SELECT id FROM snapshot_ports WHERE snapshots_id=?1)",
            [id],
        )?;
        connection.execute("DELETE FROM snapshot_ports WHERE snapshots_id=?1", [id])?;
        Ok(connection.execute("DELETE FROM snapshots WHERE id=?1", [id])? == 1)
    }
}

fn insert_port(connection: &Connection, snapshot: i64, port: &SnapshotPort) -> Result<i64> {
    ensure!(!port.port_name.is_empty(), "snapshot port without a name");
    connection.execute(
        "INSERT INTO snapshot_ports(snapshots_id, port_name, requested, state, variants, requested_variants) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot,
            port.port_name,
            port.requested,
            port.state,
            port.variants,
            port.requested_variants
        ],
    )?;
    Ok(connection.last_insert_rowid())
}
