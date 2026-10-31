//! Explicit transactional registry mutation. Filesystem activation is a separate coordinator.
use super::{
    Entry,
    db::{
        Database, Mode,
        schema::{Dependencies, Files, Metadata, Ports},
    },
    read_entries,
};
use anyhow::{Context, Result, ensure};
use sea_query::{Expr, Query};
use sqlx::Row as _;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Component, Path},
    time::{SystemTime, UNIX_EPOCH},
};

pub const SCHEMA_VERSION: &str = "1.215";
pub struct Writer {
    database: Database,
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
            let mut database = Database::open(temporary.path(), Mode::Create)?;
            database.batch(include_str!("schema.sql"))?;
            database.close()?;
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
        let mut writer = Self::open(path)?;
        writer.database.batch("PRAGMA journal_mode=WAL")?;
        Ok(writer)
    }
    /// Open only an existing current-schema database. Never silently migrate it.
    pub fn open(path: &Path) -> Result<Self> {
        // macOS exposes /var through a symlink. Resolve parent directories,
        // while refusing a database path that is itself a symlink.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let filename = path.file_name().context("registry path has no filename")?;
        let resolved = parent.canonicalize()?.join(filename);
        let mut database = Database::open(&resolved, Mode::ReadWrite)
            .with_context(|| format!("cannot open registry {} for writing", path.display()))?;
        let statement = Query::select()
            .column(Metadata::Value)
            .from(Metadata::Table)
            .and_where(Expr::col(Metadata::Key).eq("version"))
            .to_owned();
        let schema: String = database
            .select_one(&statement)
            .ok()
            .flatten()
            .context("registry schema version is missing")?
            .try_get(0)?;
        ensure!(
            schema == SCHEMA_VERSION,
            "registry writes require schema {SCHEMA_VERSION}, found {schema}; automatic migrations are not implemented"
        );
        // Check the exact columns used by mutations before starting any write.
        for (table, required) in [
            (
                "ports",
                &[
                    "id",
                    "name",
                    "portfile",
                    "location",
                    "epoch",
                    "version",
                    "revision",
                    "variants",
                    "requested_variants",
                    "state",
                    "date",
                    "installtype",
                    "archs",
                    "requested",
                    "os_platform",
                    "os_major",
                    "cxx_stdlib",
                    "cxx_stdlib_overridden",
                ][..],
            ),
            (
                "files",
                &["id", "path", "actual_path", "active", "binary"][..],
            ),
            ("dependencies", &["id", "name", "variants"][..]),
            (
                "portgroups",
                &["id", "name", "version", "size", "sha256"][..],
            ),
        ] {
            let columns: HashSet<String> = database.columns(table)?.into_iter().collect();
            ensure!(
                required.iter().all(|column| columns.contains(*column)),
                "incomplete registry schema for writing"
            );
        }
        Ok(Self { database })
    }
    pub fn transaction(&mut self) -> Result<WriteTransaction<'_>> {
        self.database.batch("BEGIN IMMEDIATE")?;
        Ok(WriteTransaction {
            database: &mut self.database,
            finished: false,
        })
    }
}
/// An immediate-mode transaction, rolled back unless committed.
pub struct WriteTransaction<'a> {
    database: &'a mut Database,
    finished: bool,
}
impl Drop for WriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.database.batch("ROLLBACK");
        }
    }
}
impl WriteTransaction<'_> {
    pub(super) fn database(&mut self) -> &mut Database {
        self.database
    }
    pub fn entries(&mut self) -> Result<Vec<Entry>> {
        read_entries(self.database)
    }
    /// Read one image's identity from the same locked snapshot used for writes.
    pub fn image_identity(&mut self, id: i64) -> Result<ImageIdentity> {
        let statement = Query::select()
            .columns([
                Ports::Name,
                Ports::Version,
                Ports::Revision,
                Ports::Epoch,
                Ports::Variants,
                Ports::Archs,
            ])
            .from(Ports::Table)
            .and_where(Expr::col(Ports::Id).eq(id))
            .to_owned();
        let row = self
            .database
            .select_one(&statement)?
            .context("image is not registered")?;
        Ok(ImageIdentity {
            name: row.try_get(0)?,
            version: row.try_get(1)?,
            revision: row.try_get(2)?,
            epoch: row.try_get(3)?,
            variants: row.try_get(4)?,
            archs: row.try_get(5)?,
        })
    }
    /// Register an inactive image, its complete file map, and runtime dependencies.
    /// A savepoint ensures a failed operation cannot leave a partial image even
    /// if the caller catches the error and commits other operations afterwards.
    pub fn register(&mut self, image: &NewImage) -> Result<i64> {
        validate(image)?;
        let date = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let date = i64::try_from(date)?;
        self.database.savepoint(|database| {
            let id = database.insert(
                Query::insert()
                    .into_table(Ports::Table)
                    .columns([
                        Ports::Name,
                        Ports::Portfile,
                        Ports::Location,
                        Ports::Epoch,
                        Ports::Version,
                        Ports::Revision,
                        Ports::Variants,
                        Ports::RequestedVariants,
                        Ports::State,
                        Ports::Date,
                        Ports::Installtype,
                        Ports::Archs,
                        Ports::Requested,
                        Ports::OsPlatform,
                        Ports::OsMajor,
                        Ports::CxxStdlib,
                        Ports::CxxStdlibOverridden,
                    ])
                    .values([
                        image.name.clone().into(),
                        image.portfile.clone().into(),
                        image.location.clone().into(),
                        image.epoch.into(),
                        image.version.clone().into(),
                        image.revision.into(),
                        image.variants.clone().into(),
                        image.requested_variants.clone().into(),
                        "imaged".into(),
                        date.into(),
                        "image".into(),
                        image.archs.clone().into(),
                        image.requested.into(),
                        image.platform.clone().into(),
                        image.os_major.clone().into(),
                        (!image.cxx_stdlib.is_empty())
                            .then(|| image.cxx_stdlib.clone())
                            .into(),
                        image.cxx_stdlib_overridden.into(),
                    ])?,
            )?;
            for record in &image.files {
                database.insert(
                    Query::insert()
                        .into_table(Files::Table)
                        .columns([
                            Files::Id,
                            Files::Path,
                            Files::ActualPath,
                            Files::Active,
                            Files::Binary,
                        ])
                        .values([
                            id.into(),
                            record.path.clone().into(),
                            Option::<String>::None.into(),
                            0.into(),
                            record.binary.into(),
                        ])?,
                )?;
            }
            let mut seen = HashSet::new();
            for name in &image.dependencies {
                if seen.insert(name.to_ascii_lowercase()) {
                    database.insert(
                        Query::insert()
                            .into_table(Dependencies::Table)
                            .columns([Dependencies::Id, Dependencies::Name, Dependencies::Variants])
                            .values([id.into(), name.clone().into(), "".into()])?,
                    )?;
                }
            }
            Ok(id)
        })
    }
    /// Validate complete file mappings and ownership under this transaction's
    /// immediate writer lock, without changing any registry records. Call this
    /// before filesystem publication; activate rechecks before recording success.
    pub fn check_activation(&mut self, id: i64, files: &[ActivationFile]) -> Result<()> {
        let database = &mut *self.database;
        let row = database
            .select_one(
                Query::select()
                    .columns([Ports::Name, Ports::State])
                    .from(Ports::Table)
                    .and_where(Expr::col(Ports::Id).eq(id)),
            )?
            .context("image is not registered")?;
        let (name, state): (String, String) = (row.try_get(0)?, row.try_get(1)?);
        ensure!(
            state == "imaged",
            "image must be inactive before activation"
        );
        let conflict = database.select_one(
            Query::select()
                .column(Ports::Name)
                .from(Ports::Table)
                .and_where(Expr::cust_with_values(
                    "\"name\" = ? COLLATE NOCASE",
                    [name.clone()],
                ))
                .and_where(Expr::col(Ports::State).eq("installed"))
                .limit(1),
        )?;
        ensure!(
            conflict.is_none(),
            "another image of {name} is already active"
        );
        let registered: Vec<String> = database
            .select(
                Query::select()
                    .column(Files::Path)
                    .from(Files::Table)
                    .and_where(Expr::col(Files::Id).eq(id)),
            )?
            .iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()?;
        let count = registered.len();
        let registered: HashSet<_> = registered.into_iter().collect();
        ensure!(registered.len() == count, "duplicate registered file map");
        let mut mappings = BTreeMap::new();
        let mut destinations = HashSet::new();
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
            for path in Path::new(&file.actual_path).ancestors() {
                let path = path.to_str().context("activation path is not UTF-8")?;
                let owner = database.select_one(
                    Query::select()
                        .column(Files::Id)
                        .from(Files::Table)
                        .and_where(Expr::col(Files::Active).ne(0))
                        .and_where(Expr::cust_with_values(
                            "\"actual_path\" = ? COLLATE NOCASE",
                            [path],
                        ))
                        .limit(1),
                )?;
                ensure!(
                    owner.is_none(),
                    "activation path {} conflicts with {} owned by another image",
                    file.actual_path,
                    path
                );
            }
            // The range covers only children of this exact path component.
            // '/' is immediately before '0' in SQLite's NOCASE ordering. Unlike
            // LIKE, literal '%' and '_' in filenames do not become wildcards.
            let descendant = database.select_one(
                Query::select()
                    .column(Files::ActualPath)
                    .from(Files::Table)
                    .and_where(Expr::col(Files::Active).ne(0))
                    .and_where(Expr::cust_with_values(
                        "\"actual_path\" >= ? COLLATE NOCASE",
                        [format!("{}/", file.actual_path)],
                    ))
                    .and_where(Expr::cust_with_values(
                        "\"actual_path\" < ? COLLATE NOCASE",
                        [format!("{}0", file.actual_path)],
                    ))
                    .limit(1),
            )?;
            if let Some(descendant) = descendant {
                let descendant: String = descendant.try_get(0)?;
                anyhow::bail!(
                    "activation path {} contains {} owned by another image",
                    file.actual_path,
                    descendant
                );
            }
        }
        for destination in &destinations {
            for parent in Path::new(destination).ancestors().skip(1) {
                ensure!(
                    !destinations
                        .contains(parent.to_str().context("activation path is not UTF-8")?),
                    "activation destinations cannot contain one another: {destination}"
                );
            }
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
        self.database.savepoint(|database| {
            for file in files {
                ensure!(
                    database.update(
                        Query::update()
                            .table(Files::Table)
                            .value(Files::ActualPath, file.actual_path.clone())
                            .value(Files::Active, 1)
                            .and_where(Expr::col(Files::Id).eq(id))
                            .and_where(Expr::col(Files::Path).eq(file.path.clone())),
                    )? == 1,
                    "duplicate registered file map"
                );
            }
            database.update(
                Query::update()
                    .table(Ports::Table)
                    .value(Ports::State, "installed")
                    .and_where(Expr::col(Ports::Id).eq(id)),
            )?;
            Ok(())
        })
    }
    pub fn deactivate(&mut self, id: i64) -> Result<()> {
        self.database.savepoint(|database| {
            ensure!(
                database.update(
                    Query::update()
                        .table(Ports::Table)
                        .value(Ports::State, "imaged")
                        .and_where(Expr::col(Ports::Id).eq(id))
                        .and_where(Expr::col(Ports::State).eq("installed")),
                )? == 1,
                "image is not active"
            );
            database.update(
                Query::update()
                    .table(Files::Table)
                    .value(Files::Active, 0)
                    .and_where(Expr::col(Files::Id).eq(id)),
            )?;
            Ok(())
        })
    }
    pub fn remove(&mut self, id: i64) -> Result<()> {
        ensure!(
            self.database.delete(
                Query::delete()
                    .from_table(Ports::Table)
                    .and_where(Expr::col(Ports::Id).eq(id))
                    .and_where(Expr::col(Ports::State).eq("imaged")),
            )? == 1,
            "only registered inactive images can be removed"
        );
        Ok(())
    }
    pub fn set_requested(&mut self, id: i64, requested: bool) -> Result<()> {
        ensure!(
            self.database.update(
                Query::update()
                    .table(Ports::Table)
                    .value(Ports::Requested, requested)
                    .and_where(Expr::col(Ports::Id).eq(id)),
            )? == 1,
            "image is not registered"
        );
        Ok(())
    }
    pub fn commit(mut self) -> Result<()> {
        self.database.batch("COMMIT")?;
        self.finished = true;
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
