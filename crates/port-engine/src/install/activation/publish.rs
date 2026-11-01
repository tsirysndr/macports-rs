use super::Plan;
use crate::{
    install::{
        inspect::Kind,
        publication::{PublishedDirectory, PublishedFile, Root},
    },
    registry::write::{ActivationFile, WriteTransaction, Writer},
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};

/// Filesystem publication pending registry activation and commit. Drop rolls back
/// in-process; the caller must keep exclusive destination access throughout.
/// This is not a durable crash-recovery journal.
pub struct PublishedImage {
    mappings: Vec<ActivationFile>,
    files: Vec<PublishedFile>,
    directories: Vec<PublishedDirectory>,
}
impl PublishedImage {
    pub fn mappings(&self) -> &[ActivationFile] {
        &self.mappings
    }
    /// Call only after registry activation and transaction commit succeed.
    pub fn persist(mut self) {
        for file in self.files.drain(..) {
            file.persist();
        }
        for directory in self.directories.drain(..) {
            directory.persist();
        }
    }
    pub fn rollback(mut self) -> Result<()> {
        self.remove()
    }
    fn remove(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        // Final archive permissions may remove directory search/write access.
        // Reopen access on our own directories before removing their children.
        for directory in &self.directories {
            if let Err(error) = directory.prepare_rollback() {
                errors.push(error.to_string());
            }
        }
        while let Some(file) = self.files.pop() {
            if let Err(error) = file.rollback() {
                errors.push(error.to_string());
            }
        }
        while let Some(directory) = self.directories.pop() {
            if let Err(error) = directory.rollback() {
                errors.push(error.to_string());
            }
        }
        ensure!(
            errors.is_empty(),
            "image rollback failed: {}",
            errors.join("; ")
        );
        Ok(())
    }
}
impl Drop for PublishedImage {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
impl Plan<'_> {
    /// Publish an already registered inactive image, activate its file mappings,
    /// and retain the files only after the registry transaction commits. The
    /// caller must exclusively manage destination paths for the entire call.
    /// Recovery from process termination still requires a durable journal.
    #[cfg(target_os = "macos")]
    pub fn activate(&self, writer: &mut Writer, image_id: i64) -> Result<()> {
        let mut transaction = writer.transaction()?;
        let published = self.publish(&mut transaction, image_id)?;
        if let Err(error) = transaction.activate(image_id, published.mappings()) {
            // Keep the registry writer lock while undoing failed activation.
            return match published.rollback() {
                Ok(()) => Err(error),
                Err(rollback) => {
                    Err(error.context(format!("activation rollback also failed: {rollback:#}")))
                }
            };
        }
        if let Err(error) = transaction.commit() {
            return match published.rollback() {
                Ok(()) => Err(error),
                Err(rollback) => {
                    Err(error.context(format!("commit rollback also failed: {rollback:#}")))
                }
            };
        }
        published.persist();
        Ok(())
    }

    /// Recheck the filesystem and locked registry, then publish payload and
    /// metadata. The caller still owns registry activation/commit and must retain
    /// the returned receipt until both succeed. Control files are never executed.
    #[cfg(target_os = "macos")]
    pub fn publish(
        &self,
        transaction: &mut WriteTransaction<'_>,
        image_id: i64,
    ) -> Result<PublishedImage> {
        let plan = Plan::inspect(self.image, &self.root)?;
        let mappings = plan.check_registry(transaction, image_id)?;
        for member in plan.image.inspection().members.values() {
            ensure!(
                member.mode <= 0o7777
                    && member.metadata.uid < u64::from(u32::MAX)
                    && member.metadata.gid < u64::from(u32::MAX),
                "invalid activation metadata"
            );
        }
        let root = Root::open(&plan.root)?;
        let mut published = PublishedImage {
            mappings,
            files: Vec::new(),
            directories: Vec::new(),
        };
        let result = (|| -> Result<()> {
            let mut directory_indices = Vec::new();
            for directory in &plan.directories {
                if let Some(receipt) =
                    root.create_directory(directory.destination.strip_prefix(&plan.root)?, 0o700)?
                {
                    directory_indices.push(directory);
                    published.directories.push(receipt);
                }
            }
            let mut originals = BTreeMap::new();
            // Hardlinks may sort before their regular-file source.
            for file in plan
                .files
                .iter()
                .filter(|file| !matches!(file.member.kind, Kind::Hardlink(_)))
            {
                let relative = file.destination.strip_prefix(&plan.root)?;
                let receipt = match &file.member.kind {
                    Kind::File => {
                        let mut input = OpenOptions::new()
                            .read(true)
                            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                            .open(&file.source)?;
                        ensure!(
                            input.metadata()?.is_file()
                                && input.metadata()?.len() == file.member.size,
                            "prepared payload is not a regular file of the expected size"
                        );
                        root.publish_file(relative, &mut input, 0o600)?
                    }
                    Kind::Symlink(target) => root.publish_symlink(relative, Path::new(target))?,
                    _ => unreachable!("plan contains only payload files"),
                };
                originals.insert(relative.to_owned(), published.files.len());
                published.files.push(receipt);
                let receipt = published.files.last().unwrap();
                let metadata = &file.member.metadata;
                receipt.set_owner_mode(metadata.uid, metadata.gid, file.member.mode)?;
                receipt.set_times(metadata.atime, metadata.mtime)?;
                receipt.set_flags(file_flags(&file.member.attributes))?;
            }
            for file in &plan.files {
                if let Kind::Hardlink(target) = &file.member.kind {
                    let index = originals
                        .get(Path::new(target))
                        .context("missing published hardlink source")?;
                    let receipt = root.publish_hardlink(
                        file.destination.strip_prefix(&plan.root)?,
                        &published.files[*index],
                    )?;
                    published.files.push(receipt);
                }
            }
            for (directory, receipt) in directory_indices.iter().zip(&published.directories).rev() {
                if let Some(name) = &directory.member_path {
                    let member = &plan.image.inspection().members[name];
                    let metadata = &member.metadata;
                    receipt.set_owner_mode(metadata.uid, metadata.gid, member.mode)?;
                    receipt.set_times(metadata.atime, metadata.mtime)?;
                    receipt.set_flags(file_flags(&member.attributes))?;
                } else {
                    // Preserve creation ownership, including inherited groups.
                    receipt.set_mode(0o755)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            if let Err(rollback) = published.remove() {
                return Err(
                    error.context(format!("publication rollback also failed: {rollback:#}"))
                );
            }
            return Err(error);
        }
        Ok(published)
    }
}

/// The chflags bits for an archive member's `SCHILY.fflags` (names as
/// strtofflags). `compressed` only marks transparent compression, which the
/// published copy does not carry, and `restricted` belongs to the system.
fn file_flags(attributes: &BTreeMap<String, String>) -> u32 {
    let Some(names) = attributes.get("SCHILY.fflags") else {
        return 0;
    };
    names
        .split(',')
        .map(|name| match name.trim() {
            "arch" | "archived" => 0x0001_0000,
            "opaque" => 0x0000_0008,
            "nodump" => 0x0000_0001,
            "sappnd" | "sappend" => 0x0004_0000,
            "schg" | "schange" | "simmutable" => 0x0002_0000,
            "sunlnk" | "sunlink" => 0x0010_0000,
            "uappnd" | "uappend" => 0x0000_0004,
            "uchg" | "uchange" | "uimmutable" => 0x0000_0002,
            "hidden" => 0x0000_8000,
            _ => 0,
        })
        .fold(0, |flags, flag| flags | flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_flags_skip_compression() {
        let flags = |value: &str| {
            file_flags(&BTreeMap::from([(
                "SCHILY.fflags".to_string(),
                value.to_string(),
            )]))
        };
        assert_eq!(flags("compressed"), 0);
        assert_eq!(flags("hidden,compressed"), 0x8000);
        assert_eq!(flags("uchg,nodump"), 0x3);
        assert_eq!(file_flags(&BTreeMap::new()), 0);
    }
}
