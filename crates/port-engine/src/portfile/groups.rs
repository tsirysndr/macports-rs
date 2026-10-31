//! Explicit PortGroup sources. Searches are local, ordered, and bounded; imports
//! use the same interpreter and never invoke Tcl or fetch code from a network.
use super::*;
use anyhow::Context;
use molt::{MoltResult, check_args};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{ErrorKind, Read},
    path::PathBuf,
};
const MAX_SOURCE: u64 = 4 * 1024 * 1024;

/// Caller-selected search directories, in priority order, plus optional embedded
/// sources (which take precedence). Filenames are `<name>-<version>.tcl`.
#[derive(Clone, Default)]
pub struct PortGroups {
    directories: Vec<PathBuf>,
    pub(super) resource_directory: Option<PathBuf>,
    embedded: BTreeMap<(String, String), String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedGroup {
    pub name: String,
    pub version: String,
    pub path: Option<PathBuf>,
    pub size: u64,
    pub sha256: String,
}
impl PortGroups {
    /// Bind local PortGroups and runtime resources to one selected ports tree.
    pub fn from_tree(root: &std::path::Path) -> Self {
        Self {
            directories: vec![root.join("_resources/port1.0/group")],
            resource_directory: Some(root.join("_resources")),
            ..Self::default()
        }
    }
    pub fn from_directories(directories: Vec<PathBuf>) -> Self {
        Self {
            directories,
            ..Self::default()
        }
    }
    pub fn insert(&mut self, name: &str, version: &str, source: String) -> Result<()> {
        validate(name, version)?;
        ensure!(
            source.len() as u64 <= MAX_SOURCE,
            "PortGroup source exceeds size limit"
        );
        self.embedded.insert((name.into(), version.into()), source);
        Ok(())
    }
    fn read(&self, name: &str, version: &str) -> Result<(String, LoadedGroup)> {
        validate(name, version)?;
        let key = (name.to_owned(), version.to_owned());
        let (source, path) = if let Some(source) = self.embedded.get(&key) {
            (source.clone(), None)
        } else {
            let mut found = None;
            for directory in &self.directories {
                let path = directory.join(format!("{name}-{version}.tcl"));
                let metadata = match fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("cannot read PortGroup {}", path.display()));
                    }
                };
                ensure!(
                    metadata.is_file(),
                    "PortGroup {} is not a regular file",
                    path.display()
                );
                ensure!(
                    metadata.len() <= MAX_SOURCE,
                    "PortGroup source exceeds size limit"
                );
                let file = File::open(&path)
                    .with_context(|| format!("cannot open PortGroup {}", path.display()))?;
                ensure!(
                    file.metadata()?.is_file(),
                    "PortGroup is not a regular file"
                );
                let mut source = String::new();
                file.take(MAX_SOURCE + 1)
                    .read_to_string(&mut source)
                    .with_context(|| format!("cannot read PortGroup {}", path.display()))?;
                ensure!(
                    source.len() as u64 <= MAX_SOURCE,
                    "PortGroup source exceeds size limit"
                );
                found = Some((source, Some(path)));
                break;
            }
            found.with_context(|| {
                format!("PortGroup {name} {version} not found in configured sources")
            })?
        };
        let record = LoadedGroup {
            name: name.into(),
            version: version.into(),
            path,
            size: source.len() as u64,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        };
        Ok((source, record))
    }
}
fn validate(name: &str, version: &str) -> Result<()> {
    for value in [name, version] {
        ensure!(
            !value.is_empty()
                && !matches!(value, "." | "..")
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c)),
            "invalid PortGroup name or version"
        );
    }
    Ok(())
}
pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "name version")?;
    match import(interp, id, argv) {
        Ok(value) => Ok(value),
        Err(failure) => {
            let message = format!("PortGroup {} {}: {}", argv[1], argv[2], failure.value());
            interp
                .context::<State>(id)
                .group_failures
                .insert(message.clone());
            Err(error(message))
        }
    }
}
fn import(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let name = argv[1].as_str();
    let version = argv[2].as_str();
    let key = (name.to_owned(), version.to_owned());
    let state = interp.context::<State>(id);
    if state
        .loaded_groups
        .iter()
        .any(|group| group.name == key.0 && group.version == key.1)
    {
        return Ok(Value::empty());
    }
    let (source, record) = state
        .groups
        .read(name, version)
        .map_err(|failure| error(format!("{failure:#}")))?;
    // Upstream records the import before evaluating it. Recursive imports and
    // repeated inclusions of the same version therefore execute only once.
    state.loaded_groups.push(record);
    let records = state
        .loaded_groups
        .iter()
        .map(|group| {
            list([
                group.name.clone(),
                group.version.clone(),
                group
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<embedded>".into()),
            ])
            .to_string()
        })
        .collect::<Vec<_>>();
    let path = interp
        .context::<State>(id)
        .loaded_groups
        .last()
        .and_then(|group| group.path.as_ref())
        .map(|path| path.to_string_lossy().into_owned());
    global(interp, "PortInfo");
    interp.set_element("PortInfo", "portgroups", list(records))?;
    // Like `source`, a group's top-level `return` ends only that group.
    let result = match &path {
        Some(path) => interp.eval_file(path, &source),
        None => interp.eval(&source),
    };
    match result {
        Err(exception) if exception.code() == molt::ResultCode::Return => Ok(exception.value()),
        other => other,
    }
}
