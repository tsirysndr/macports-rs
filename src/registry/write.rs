//! Explicit transactional registry mutation. Filesystem activation is a separate coordinator.
use super::{Entry, read_entries, version};
use anyhow::{Context, Result, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Component, Path},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const SCHEMA_VERSION: &str = "1.215";
pub struct Writer {
    connection: Connection,
}
#[derive(Default)]
pub struct NewImage {
    pub name: String,
    pub version: String,
    pub revision: i64,
    pub epoch: i64,
    pub variants: String,
    pub requested_variants: String,
    pub requested: bool,
    pub location: String,
    pub portfile: String,
    pub platform: String,
    pub os_major: String,
    pub archs: String,
    pub cxx_stdlib: String,
    pub cxx_stdlib_overridden: bool,
    pub files: Vec<ImageFile>,
    pub dependencies: Vec<String>,
}
pub struct ImageFile {
    pub path: String,
    pub binary: bool,
}
pub struct ImageIdentity {
    pub name: String,
    pub version: String,
    pub revision: i64,
    pub epoch: i64,
    pub variants: String,
    pub archs: String,
}
pub struct ActivationFile {
    pub path: String,
    pub actual_path: String,
}

fn configure(connection: &Connection) -> Result<()> {
    connection.create_collation("VERSION", version::compare)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA fullfsync=ON; PRAGMA synchronous=FULL;")?;
    Ok(())
}
impl Writer {
    /// Initialize a new database atomically, never overwrite an existing path.
    /// The parent directory must already exist; no implicit prefix initialization.
    pub fn create(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        {
            let connection = Connection::open(temporary.path())?;
            configure(&connection)?;
            connection.execute_batch(include_str!("schema.sql"))?;
            connection.close().map_err(|(_, error)| error)?;
        }
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)
            .with_context(|| {
                format!(
                    "cannot create registry {} without overwriting",
                    path.display()
                )
            })?;
        fs::File::open(parent)?.sync_all()?;
        let writer = Self::open(path)?;
        writer.connection.execute_batch("PRAGMA journal_mode=WAL")?;
        Ok(writer)
    }
    /// Open only an existing current-schema database. Never silently migrate it.
    pub fn open(path: &Path) -> Result<Self> {
        // macOS exposes /var through a symlink. Resolve parent directories,
        // while leaving the database itself subject to SQLite's NOFOLLOW check.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let filename = path.file_name().context("registry path has no filename")?;
        let resolved = parent.canonicalize()?.join(filename);
        let connection = Connection::open_with_flags(
            &resolved,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .with_context(|| format!("cannot open registry {} for writing", path.display()))?;
        configure(&connection)?;
        let schema: String = connection
            .query_row(
                "SELECT value FROM metadata WHERE key='version'",
                [],
                |row| row.get(0),
            )
            .context("registry schema version is missing")?;
        ensure!(
            schema == SCHEMA_VERSION,
            "registry writes require schema {SCHEMA_VERSION}, found {schema}; automatic migrations are not implemented"
        );
        // Compile the exact columns used by mutations before starting any write.
        for sql in [
            "SELECT id,name,portfile,location,epoch,version,revision,variants,requested_variants,state,date,installtype,archs,requested,os_platform,os_major,cxx_stdlib,cxx_stdlib_overridden FROM ports LIMIT 0",
            "SELECT id,path,actual_path,active,binary FROM files LIMIT 0",
            "SELECT id,name,variants FROM dependencies LIMIT 0",
            "SELECT id,name,version,size,sha256 FROM portgroups LIMIT 0",
        ] {
            connection
                .prepare(sql)
                .context("incomplete registry schema for writing")?;
        }
        Ok(Self { connection })
    }
    pub fn transaction(&mut self) -> Result<WriteTransaction<'_>> {
        Ok(WriteTransaction {
            transaction: self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?,
        })
    }
}
pub struct WriteTransaction<'a> {
    transaction: Transaction<'a>,
}
impl WriteTransaction<'_> {
    pub fn entries(&self) -> Result<Vec<Entry>> {
        read_entries(&self.transaction)
    }
    /// Read one image's identity from the same locked snapshot used for writes.
    pub fn image_identity(&self, id: i64) -> Result<ImageIdentity> {
        self.transaction
            .query_row(
                "SELECT name,version,revision,epoch,variants,archs FROM ports WHERE id=?1",
                [id],
                |row| {
                    Ok(ImageIdentity {
                        name: row.get(0)?,
                        version: row.get(1)?,
                        revision: row.get(2)?,
                        epoch: row.get(3)?,
                        variants: row.get(4)?,
                        archs: row.get(5)?,
                    })
                },
            )
            .context("image is not registered")
    }
    /// Register an inactive image, its complete file map, and runtime dependencies.
    /// A savepoint ensures a failed operation cannot leave a partial image even
    /// if the caller catches the error and commits other operations afterwards.
    pub fn register(&mut self, image: &NewImage) -> Result<i64> {
        validate(image)?;
        let savepoint = self.transaction.savepoint()?;
        let date = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let date = i64::try_from(date)?;
        savepoint.execute("INSERT INTO ports(name,portfile,location,epoch,version,revision,variants,requested_variants,state,date,installtype,archs,requested,os_platform,os_major,cxx_stdlib,cxx_stdlib_overridden) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'imaged',?9,'image',?10,?11,?12,?13,?14,?15)",
            params![image.name,image.portfile,image.location,image.epoch,image.version,image.revision,image.variants,image.requested_variants,date,image.archs,image.requested,image.platform,image.os_major,image.cxx_stdlib,image.cxx_stdlib_overridden])?;
        let id = savepoint.last_insert_rowid();
        {
            let mut file = savepoint.prepare(
                "INSERT INTO files(id,path,actual_path,active,binary) VALUES(?1,?2,NULL,0,?3)",
            )?;
            for record in &image.files {
                file.execute(params![id, record.path, record.binary])?;
            }
            let mut dependency =
                savepoint.prepare("INSERT INTO dependencies(id,name,variants) VALUES(?1,?2,'')")?;
            let mut seen = HashSet::new();
            for name in &image.dependencies {
                if seen.insert(name.to_ascii_lowercase()) {
                    dependency.execute(params![id, name])?;
                }
            }
        }
        savepoint.commit()?;
        Ok(id)
    }
    /// Validate complete file mappings and ownership under this transaction's
    /// immediate writer lock, without changing any registry records. Call this
    /// before filesystem publication; activate rechecks before recording success.
    pub fn check_activation(&self, id: i64, files: &[ActivationFile]) -> Result<()> {
        let (name, state): (String, String) = self
            .transaction
            .query_row("SELECT name,state FROM ports WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .context("image is not registered")?;
        ensure!(
            state == "imaged",
            "image must be inactive before activation"
        );
        let conflict: Option<String> = self
            .transaction
            .query_row(
                "SELECT name FROM ports WHERE name=?1 COLLATE NOCASE AND state='installed' LIMIT 1",
                [&name],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            conflict.is_none(),
            "another image of {name} is already active"
        );
        let registered: Vec<String> = self
            .transaction
            .prepare("SELECT path FROM files WHERE id=?1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let count = registered.len();
        let registered: HashSet<_> = registered.into_iter().collect();
        ensure!(registered.len() == count, "duplicate registered file map");
        let mut mappings = BTreeMap::new();
        let mut destinations = HashSet::new();
        let mut owner_query = self.transaction.prepare(
            "SELECT id FROM files WHERE active AND actual_path=?1 COLLATE NOCASE LIMIT 1",
        )?;
        for file in files {
            absolute_path(&file.path)?;
            absolute_path(&file.actual_path)?;
            ensure!(
                mappings.insert(&file.path, &file.actual_path).is_none(),
                "duplicate activation source"
            );
            ensure!(
                destinations.insert(file.actual_path.to_ascii_lowercase()),
                "duplicate activation destination"
            );
            let owner: Option<i64> = owner_query
                .query_row([&file.actual_path], |r| r.get(0))
                .optional()?;
            ensure!(
                owner.is_none(),
                "activation path {} is owned by another image",
                file.actual_path
            );
        }
        ensure!(
            registered.len() == mappings.len()
                && registered.iter().all(|path| mappings.contains_key(path)),
            "activation mapping must cover exactly the registered image files"
        );
        Ok(())
    }
    /// Record activation after the filesystem coordinator published these files.
    /// This method itself does not create or replace filesystem entries.
    pub fn activate(&mut self, id: i64, files: &[ActivationFile]) -> Result<()> {
        self.check_activation(id, files)?;
        let savepoint = self.transaction.savepoint()?;
        {
            let mut update = savepoint
                .prepare("UPDATE files SET actual_path=?1,active=1 WHERE id=?2 AND path=?3")?;
            for file in files {
                ensure!(
                    update.execute(params![file.actual_path, id, file.path])? == 1,
                    "duplicate registered file map"
                );
            }
        }
        savepoint.execute("UPDATE ports SET state='installed' WHERE id=?1", [id])?;
        savepoint.commit()?;
        Ok(())
    }
    pub fn deactivate(&mut self, id: i64) -> Result<()> {
        let savepoint = self.transaction.savepoint()?;
        ensure!(
            savepoint.execute(
                "UPDATE ports SET state='imaged' WHERE id=?1 AND state='installed'",
                [id]
            )? == 1,
            "image is not active"
        );
        savepoint.execute("UPDATE files SET active=0 WHERE id=?1", [id])?;
        savepoint.commit()?;
        Ok(())
    }
    pub fn remove(&mut self, id: i64) -> Result<()> {
        ensure!(
            self.transaction
                .execute("DELETE FROM ports WHERE id=?1 AND state='imaged'", [id])?
                == 1,
            "only registered inactive images can be removed"
        );
        Ok(())
    }
    pub fn set_requested(&mut self, id: i64, requested: bool) -> Result<()> {
        ensure!(
            self.transaction.execute(
                "UPDATE ports SET requested=?1 WHERE id=?2",
                params![requested, id]
            )? == 1,
            "image is not registered"
        );
        Ok(())
    }
    pub fn commit(self) -> Result<()> {
        self.transaction.commit()?;
        Ok(())
    }
}
fn port_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+'))
}
fn absolute_path(path: &str) -> Result<()> {
    ensure!(
        Path::new(path).is_absolute()
            && path != "/"
            && !path.contains('\\')
            && !path.chars().any(char::is_control)
            && !path.split('/').any(|part| matches!(part, "." | ".."))
            && Path::new(path)
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "registry file paths must be normalized and absolute"
    );
    ensure!(
        !path.contains("//") && !path.ends_with('/'),
        "registry file paths must be normalized and absolute"
    );
    Ok(())
}
fn validate(image: &NewImage) -> Result<()> {
    ensure!(port_name(&image.name), "invalid installed port name");
    ensure!(
        !image.version.is_empty()
            && !image
                .version
                .chars()
                .any(|c| c.is_control() || c.is_whitespace()),
        "invalid installed port version"
    );
    ensure!(
        image.epoch >= 0 && image.revision >= 0,
        "negative epoch or revision"
    );
    absolute_path(&image.location)?;
    for (value, positive_only) in [(&image.variants, true), (&image.requested_variants, false)] {
        if !value.is_empty() {
            let mut spec = crate::spec::PortSpec::named(&image.name);
            spec.parse_modifier(value)?;
            ensure!(
                spec.options.is_empty()
                    && spec.version.is_none()
                    && (!positive_only || spec.variants.values().all(|enabled| *enabled))
                    && spec.composite_version() == *value,
                "registry variants must be canonical"
            );
        }
    }
    let mut paths = HashSet::new();
    for file in &image.files {
        absolute_path(&file.path)?;
        ensure!(
            paths.insert(file.path.to_ascii_lowercase()),
            "duplicate image file path"
        );
    }
    ensure!(
        image.dependencies.iter().all(|name| port_name(name)),
        "invalid dependency port name"
    );
    Ok(())
}
