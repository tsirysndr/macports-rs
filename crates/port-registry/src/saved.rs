//! Portfiles and PortGroups saved beside the registry database, as upstream's
//! `portimage::install` keeps them: `portfiles/<name>-<version>_<revision>/
//! <sha256>-<size>/Portfile` and `portgroups/<sha256>-<size>/<name>-<version>.tcl`.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedGroup {
    pub name: String,
    pub version: String,
    pub size: i64,
    pub sha256: String,
}

impl SavedGroup {
    pub fn directory(&self, registry: &Path) -> PathBuf {
        registry
            .join("portgroups")
            .join(format!("{}-{}", self.sha256, self.size))
    }
    fn file(&self, registry: &Path) -> PathBuf {
        self.directory(registry)
            .join(format!("{}-{}.tcl", self.name, self.version))
    }
}

/// The directory holding the registry database, which upstream calls
/// `${registry.path}/registry`.
pub fn registry_directory(database: &Path) -> &Path {
    database.parent().unwrap_or(Path::new("."))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Whether a `portfile` column holds a saved-Portfile key rather than text.
pub fn is_key(portfile: &str) -> bool {
    portfile.split_once('-').is_some_and(|(sha, size)| {
        sha.len() == 64
            && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            && size.parse::<u64>().is_ok()
    })
}

pub fn portfile_directory(
    registry: &Path,
    name: &str,
    version: &str,
    revision: i64,
    key: &str,
) -> PathBuf {
    registry
        .join("portfiles")
        .join(format!("{name}-{version}_{revision}"))
        .join(key)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    let directory = path.parent().context("saved file has no directory")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;
    fs::write(path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    Ok(())
}

/// Save a Portfile and return its `<sha256>-<size>` key.
pub fn save_portfile(
    registry: &Path,
    name: &str,
    version: &str,
    revision: i64,
    portfile: &Path,
) -> Result<String> {
    let bytes =
        fs::read(portfile).with_context(|| format!("cannot read {}", portfile.display()))?;
    let key = format!("{}-{}", digest(&bytes), bytes.len());
    write_if_changed(
        &portfile_directory(registry, name, version, revision, &key).join("Portfile"),
        &bytes,
    )?;
    Ok(key)
}

pub fn save_group(registry: &Path, name: &str, version: &str, path: &Path) -> Result<SavedGroup> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let group = SavedGroup {
        name: name.into(),
        version: version.into(),
        size: bytes.len() as i64,
        sha256: digest(&bytes),
    };
    write_if_changed(&group.file(registry), &bytes)?;
    Ok(group)
}

pub fn remove_portfile(registry: &Path, name: &str, version: &str, revision: i64, key: &str) {
    let directory = portfile_directory(registry, name, version, revision, key);
    let _ = fs::remove_dir_all(&directory);
    if let Some(parent) = directory.parent() {
        let _ = fs::remove_dir(parent);
    }
}

pub fn remove_group(registry: &Path, group: &SavedGroup) {
    let _ = fs::remove_file(group.file(registry));
    let _ = fs::remove_dir(group.directory(registry));
}
