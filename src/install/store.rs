//! Reversible storage of authenticated archives and their signature sidecars.
use super::{
    Item,
    archive::VerifiedArchive,
    inspect::{self, Inspection, Limits},
    publication::{PublishedDirectory, PublishedFile, Root},
    target::Target,
};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

pub struct Store {
    root: Root,
    path: PathBuf,
}
impl Store {
    /// Open an existing physical software directory. The caller must exclusively
    /// manage it until every returned storage receipt is retained or rolled back.
    pub fn open(path: &Path) -> Result<Self> {
        let root = Root::open(path)?;
        Ok(Self {
            root,
            path: path.canonicalize()?,
        })
    }
    /// Validate identity and architecture before creating a per-port directory.
    /// Neither an existing archive nor an existing signature is overwritten.
    pub fn stage(
        &self,
        archive: &mut VerifiedArchive,
        item: &Item<'_>,
        target: &Target,
        limits: Limits,
    ) -> Result<StoredImage> {
        let filename = target.filename(item)?;
        let inspection = inspect::tbz2(archive, limits)?;
        inspection.contents.matches(item)?;
        target.matches(&inspection.contents)?;
        let relative = Path::new(&item.port.name).join(filename);
        let signature_relative = PathBuf::from(format!("{}.sig", relative.display()));
        let mut stored = StoredImage {
            location: self.path.join(&relative),
            signature_location: self.path.join(&signature_relative),
            inspection,
            files: Vec::new(),
            directory: self
                .root
                .create_directory(Path::new(&item.port.name), 0o755)?,
        };
        let result = (|| -> Result<()> {
            stored.files.push(
                self.root
                    .publish_file(&relative, &mut archive.reader()?, 0o644)?,
            );
            stored.files.push(self.root.publish_file(
                &signature_relative,
                &mut archive.signature().as_bytes(),
                0o644,
            )?);
            Ok(())
        })();
        if let Err(error) = result {
            if let Err(rollback) = stored.remove() {
                return Err(error.context(format!(
                    "archive storage rollback also failed: {rollback:#}"
                )));
            }
            return Err(error);
        }
        Ok(stored)
    }
}

/// An image-store addition pending registry registration. Retain it only after
/// committing registration; dropping it removes only this addition's inodes.
pub struct StoredImage {
    location: PathBuf,
    signature_location: PathBuf,
    inspection: Inspection,
    files: Vec<PublishedFile>,
    directory: Option<PublishedDirectory>,
}
impl StoredImage {
    pub fn location(&self) -> &Path {
        &self.location
    }
    pub fn signature_location(&self) -> &Path {
        &self.signature_location
    }
    pub fn inspection(&self) -> &Inspection {
        &self.inspection
    }
    pub fn persist(mut self) {
        for file in self.files.drain(..) {
            file.persist();
        }
        if let Some(directory) = self.directory.take() {
            directory.persist();
        }
    }
    pub fn rollback(mut self) -> Result<()> {
        self.remove()
    }
    fn remove(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        while let Some(file) = self.files.pop() {
            if let Err(error) = file.rollback() {
                errors.push(error.to_string());
            }
        }
        if let Some(directory) = self.directory.take()
            && let Err(error) = directory.rollback()
        {
            errors.push(error.to_string());
        }
        ensure!(
            errors.is_empty(),
            "archive storage rollback failed: {}",
            errors.join("; ")
        );
        Ok(())
    }
}
impl Drop for StoredImage {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
