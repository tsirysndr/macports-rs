//! Prepare an authenticated image in a private directory, never the live prefix.
use super::{
    Item,
    archive::VerifiedArchive,
    inspect::{self, Inspection, Kind, Limits},
};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{OpenOptionsExt, symlink},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

pub struct PreparedImage {
    directory: tempfile::TempDir,
    inspection: Arc<Inspection>,
    archive_digest: [u8; 32],
    files: OnceLock<Vec<crate::registry::write::ImageFile>>,
}
impl PreparedImage {
    pub fn inspection_for(&self, archive: &VerifiedArchive) -> Result<Arc<Inspection>> {
        ensure!(
            self.archive_digest == archive.verification().sha256,
            "prepared image does not match authenticated archive bytes"
        );
        Ok(Arc::clone(&self.inspection))
    }
    /// Build native registry paths and binary classifications from the private
    /// image. Symlinks are recorded but never opened; hardlinks classify their
    /// regular inode, and archive control files/directories are excluded.
    pub fn registry_files(&self) -> Result<Vec<crate::registry::write::ImageFile>> {
        if let Some(files) = self.files.get() {
            return Ok(files.clone());
        }
        let files = self.classify()?;
        Ok(self.files.get_or_init(|| files).clone())
    }
    fn classify(&self) -> Result<Vec<crate::registry::write::ImageFile>> {
        let root = super::publication::Root::open(&self.payload())?;
        let mut files = Vec::new();
        for (name, member) in &self.inspection.members {
            if name.is_empty() || name.starts_with('+') || member.kind == Kind::Directory {
                continue;
            }
            let binary = match member.kind {
                Kind::File | Kind::Hardlink(_) => {
                    let mut file = root.open_file(Path::new(name))?;
                    crate::macho::is_binary(&mut file)
                        .with_context(|| format!("cannot classify image file {name}"))?
                }
                Kind::Symlink(_) => false,
                Kind::Directory => unreachable!(),
            };
            files.push(crate::registry::write::ImageFile {
                path: format!("/{name}"),
                binary,
            });
        }
        Ok(files)
    }
    pub fn payload(&self) -> PathBuf {
        self.directory.path().join("payload")
    }
    pub fn controls(&self) -> PathBuf {
        self.directory.path().join("controls")
    }
    pub fn inspection(&self) -> &Inspection {
        &self.inspection
    }
}

/// Inspect and bind package identity before creating any output. Extract only
/// into a fresh owner-private temporary directory, deleted on error or drop.
/// File modes, ownership and flags are retained as metadata for activation;
/// staged files are non-executable, owner-readable/writable regular files.
pub fn prepare(
    archive: &mut VerifiedArchive,
    expected: &Item<'_>,
    directory: &Path,
    limits: Limits,
) -> Result<PreparedImage> {
    prepare_checked(archive, expected, None, directory, limits)
}

/// Prepare for a resolved build target, rejecting architecture mismatches before
/// any filesystem output. The archive's platform is not recorded in +CONTENTS.
pub fn prepare_for_target(
    archive: &mut VerifiedArchive,
    expected: &Item<'_>,
    target: &super::target::Target,
    directory: &Path,
    limits: Limits,
) -> Result<PreparedImage> {
    prepare_checked(archive, expected, Some(target), directory, limits)
}
/// Accept an image prepared ahead of time only for the same authenticated
/// archive, port and build target.
pub fn reuse_for_target(
    prepared: PreparedImage,
    archive: &VerifiedArchive,
    expected: &Item<'_>,
    target: &super::target::Target,
) -> Result<PreparedImage> {
    let inspection = prepared.inspection_for(archive)?;
    inspection.contents.matches(expected)?;
    target.matches(&inspection.contents)?;
    Ok(prepared)
}
fn prepare_checked(
    archive: &mut VerifiedArchive,
    expected: &Item<'_>,
    target: Option<&super::target::Target>,
    directory: &Path,
    limits: Limits,
) -> Result<PreparedImage> {
    let inspection = inspect::tbz2(archive, limits)?;
    inspection.contents.matches(expected)?;
    if let Some(target) = target {
        target.matches(&inspection.contents)?;
    }
    let prepared = PreparedImage {
        directory: tempfile::Builder::new()
            .prefix("port-rs-image-")
            .tempdir_in(directory)?,
        inspection: Arc::new(inspection),
        archive_digest: archive.verification().sha256,
        files: OnceLock::new(),
    };
    let payload = prepared.payload();
    let controls = prepared.controls();
    fs::create_dir(&payload)?;
    fs::create_dir(&controls)?;
    // Build all directories before links. No operation follows a link created
    // from the archive, including when its target is an absolute outside path.
    for (name, member) in &prepared.inspection.members {
        if name.is_empty() || name.starts_with('+') {
            continue;
        }
        let destination = payload.join(name);
        let parent = if member.kind == Kind::Directory {
            destination.as_path()
        } else {
            destination.parent().context("missing image parent")?
        };
        fs::create_dir_all(parent)?;
    }
    let mut files: Vec<_> = prepared
        .inspection
        .members
        .iter()
        .filter(|(_, member)| member.kind == Kind::File)
        .collect();
    files.sort_by_key(|(_, member)| member.data_offset);
    let mut source = bzip2::read::BzDecoder::new(archive.reader()?);
    let mut position = 0;
    for (name, member) in files {
        ensure!(
            member.data_offset >= position,
            "overlapping archive file data"
        );
        let skip = member.data_offset - position;
        ensure!(
            io::copy(&mut source.by_ref().take(skip), &mut io::sink())? == skip,
            "truncated archive before {name:?}"
        );
        let destination = if name.starts_with('+') {
            controls.join(name)
        } else {
            payload.join(name)
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)
            .with_context(|| format!("cannot stage archive file {name:?}"))?;
        ensure!(
            io::copy(&mut source.by_ref().take(member.size), &mut file)? == member.size,
            "truncated archive file {name:?}"
        );
        position = member
            .data_offset
            .checked_add(member.size)
            .context("archive offset overflow")?;
    }
    for (name, member) in &prepared.inspection.members {
        if let Kind::Hardlink(target) = &member.kind {
            fs::hard_link(payload.join(target), payload.join(name))?;
        }
    }
    for (name, member) in &prepared.inspection.members {
        if let Kind::Symlink(target) = &member.kind {
            symlink(target, payload.join(name))?;
        }
    }
    Ok(prepared)
}
