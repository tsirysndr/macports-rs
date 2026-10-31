//! Registry snapshots (registry2.0 `registry::snapshot`): the active ports,
//! their requested state and variants, and their files at a point in time.
use super::{
    Registry,
    db::{
        Database,
        schema::{Files, Ports, SnapshotFiles, SnapshotPorts, Snapshots},
    },
    write::WriteTransaction,
};
use anyhow::{Result, ensure};
use sea_query::{Expr, Func, Order, Query};
use sqlx::Row as _;

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

fn text(column: impl sea_query::IntoColumnRef) -> sea_query::SimpleExpr {
    Func::coalesce([Expr::col(column).into(), Expr::val("").into()]).into()
}

impl Registry {
    /// Newest first.
    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let statement = Query::select()
            .column(Snapshots::Id)
            .column(Snapshots::CreatedAt)
            .expr(text(Snapshots::Note))
            .from(Snapshots::Table)
            .order_by(Snapshots::Id, Order::Desc)
            .to_owned();
        self.database()
            .select(&statement)?
            .iter()
            .map(|row| {
                Ok(Snapshot {
                    id: row.try_get(0)?,
                    created_at: row.try_get(1)?,
                    note: row.try_get(2)?,
                })
            })
            .collect()
    }
    pub fn snapshot(&self, id: i64) -> Result<Option<Snapshot>> {
        Ok(self
            .snapshots()?
            .into_iter()
            .find(|snapshot| snapshot.id == id))
    }
    /// Ports in a snapshot, ordered by name; files are filled in on request.
    pub fn snapshot_ports(&self, id: i64, include_files: bool) -> Result<Vec<SnapshotPort>> {
        let mut database = self.database();
        let statement = Query::select()
            .column(SnapshotPorts::Id)
            .column(SnapshotPorts::PortName)
            .expr(Func::coalesce([
                Expr::col(SnapshotPorts::Requested).into(),
                Expr::val(0).into(),
            ]))
            .expr(text(SnapshotPorts::State))
            .expr(text(SnapshotPorts::Variants))
            .expr(text(SnapshotPorts::RequestedVariants))
            .from(SnapshotPorts::Table)
            .and_where(Expr::col(SnapshotPorts::SnapshotsId).eq(id))
            .order_by(SnapshotPorts::PortName, Order::Asc)
            .to_owned();
        let rows = database.select(&statement)?;
        let mut ports = Vec::new();
        for row in rows {
            let row_id: i64 = row.try_get(0)?;
            let mut port = SnapshotPort {
                port_name: row.try_get(1)?,
                requested: row.try_get(2)?,
                state: row.try_get(3)?,
                variants: row.try_get(4)?,
                requested_variants: row.try_get(5)?,
                port_files: Vec::new(),
            };
            if include_files {
                port.port_files = database
                    .select(
                        Query::select()
                            .column(SnapshotFiles::Path)
                            .from(SnapshotFiles::Table)
                            .and_where(Expr::col(SnapshotFiles::Id).eq(row_id))
                            .order_by(SnapshotFiles::Path, Order::Asc),
                    )?
                    .iter()
                    .map(|row| row.try_get(0))
                    .collect::<Result<_, _>>()?;
            }
            ports.push(port);
        }
        Ok(ports)
    }
    /// The port in a snapshot that owned a file.
    pub fn snapshot_file_owner(&self, path: &str, id: i64) -> Result<Option<String>> {
        let statement = Query::select()
            .column((SnapshotPorts::Table, SnapshotPorts::PortName))
            .from(SnapshotPorts::Table)
            .inner_join(
                SnapshotFiles::Table,
                Expr::col((SnapshotFiles::Table, SnapshotFiles::Id))
                    .equals((SnapshotPorts::Table, SnapshotPorts::Id)),
            )
            .and_where(Expr::col((SnapshotFiles::Table, SnapshotFiles::Path)).eq(path))
            .and_where(Expr::col((SnapshotPorts::Table, SnapshotPorts::SnapshotsId)).eq(id))
            .limit(1)
            .to_owned();
        self.database()
            .select_one(&statement)?
            .map(|row| row.try_get(0).map_err(Into::into))
            .transpose()
    }
}

impl WriteTransaction<'_> {
    /// Record every active port with its files.
    pub fn create_snapshot(&mut self, note: &str) -> Result<i64> {
        let database = self.database();
        let snapshot = database.insert(
            Query::insert()
                .into_table(Snapshots::Table)
                .columns([Snapshots::Note])
                .values([note.into()])?,
        )?;
        let rows = database.select(
            Query::select()
                .column(Ports::Id)
                .column(Ports::Name)
                .expr(Func::coalesce([
                    Expr::col(Ports::Requested).into(),
                    Expr::val(0).into(),
                ]))
                .column(Ports::State)
                .expr(text(Ports::Variants))
                .expr(text(Ports::RequestedVariants))
                .from(Ports::Table)
                .and_where(Expr::col(Ports::State).eq("installed"))
                .order_by(Ports::Name, Order::Asc),
        )?;
        for row in rows {
            let id: i64 = row.try_get(0)?;
            let port = SnapshotPort {
                port_name: row.try_get(1)?,
                requested: row.try_get(2)?,
                state: row.try_get(3)?,
                variants: row.try_get(4)?,
                requested_variants: row.try_get(5)?,
                port_files: Vec::new(),
            };
            let port_row = insert_port(database, snapshot, &port)?;
            let files: Vec<String> = database
                .select(
                    Query::select()
                        .column(Files::Path)
                        .from(Files::Table)
                        .and_where(Expr::col(Files::Id).eq(id)),
                )?
                .iter()
                .map(|row| row.try_get(0))
                .collect::<Result<_, _>>()?;
            for path in files {
                insert_file(database, port_row, &path)?;
            }
        }
        Ok(snapshot)
    }
    pub fn import_snapshot(
        &mut self,
        created_at: &str,
        note: &str,
        ports: &[SnapshotPort],
    ) -> Result<i64> {
        let database = self.database();
        let snapshot = database.insert(
            Query::insert()
                .into_table(Snapshots::Table)
                .columns([Snapshots::CreatedAt, Snapshots::Note])
                .values([created_at.into(), note.into()])?,
        )?;
        for port in ports {
            let row = insert_port(database, snapshot, port)?;
            for path in &port.port_files {
                insert_file(database, row, path)?;
            }
        }
        Ok(snapshot)
    }
    /// Delete a snapshot with its ports and files; false when it does not exist.
    pub fn delete_snapshot(&mut self, id: i64) -> Result<bool> {
        let database = self.database();
        database.delete(
            Query::delete().from_table(SnapshotFiles::Table).and_where(
                Expr::col(SnapshotFiles::Id).in_subquery(
                    Query::select()
                        .column(SnapshotPorts::Id)
                        .from(SnapshotPorts::Table)
                        .and_where(Expr::col(SnapshotPorts::SnapshotsId).eq(id))
                        .to_owned(),
                ),
            ),
        )?;
        database.delete(
            Query::delete()
                .from_table(SnapshotPorts::Table)
                .and_where(Expr::col(SnapshotPorts::SnapshotsId).eq(id)),
        )?;
        Ok(database.delete(
            Query::delete()
                .from_table(Snapshots::Table)
                .and_where(Expr::col(Snapshots::Id).eq(id)),
        )? == 1)
    }
}

fn insert_port(database: &mut Database, snapshot: i64, port: &SnapshotPort) -> Result<i64> {
    ensure!(!port.port_name.is_empty(), "snapshot port without a name");
    database.insert(
        Query::insert()
            .into_table(SnapshotPorts::Table)
            .columns([
                SnapshotPorts::SnapshotsId,
                SnapshotPorts::PortName,
                SnapshotPorts::Requested,
                SnapshotPorts::State,
                SnapshotPorts::Variants,
                SnapshotPorts::RequestedVariants,
            ])
            .values([
                snapshot.into(),
                port.port_name.clone().into(),
                port.requested.into(),
                port.state.clone().into(),
                port.variants.clone().into(),
                port.requested_variants.clone().into(),
            ])?,
    )
}

fn insert_file(database: &mut Database, port: i64, path: &str) -> Result<i64> {
    database.insert(
        Query::insert()
            .into_table(SnapshotFiles::Table)
            .columns([SnapshotFiles::Id, SnapshotFiles::Path])
            .values([port.into(), path.into()])?,
    )
}
