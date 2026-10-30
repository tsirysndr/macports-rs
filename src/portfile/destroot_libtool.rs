//! Libtool archive cleanup, one component of the destroot finish lifecycle.
use super::*;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LibtoolCleanup {
    pub removed: usize,
    pub rewritten: usize,
}
impl StageContext<'_> {
    /// Clean staged libtool archives after ownership preparation. This does not
    /// run the remaining destroot_finish steps or mark the target complete.
    pub fn clean_destroot_libtool(&mut self) -> Result<LibtoolCleanup> {
        check_supported(self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "destroot cleanup requires an execution-enabled session"
        );
        let root = PathBuf::from(self.scalar("destroot")?);
        let delete = self
            .interp
            .scalar("destroot.delete_la_files")
            .map_err(failure)?
            .as_bool()
            .map_err(failure)?;
        let mut candidates = Vec::new();
        traverse::visit_paths(vec![root.clone()], |path| {
            if path.extension().is_some_and(|extension| extension == "la") {
                candidates.push(path.to_owned());
            }
            Ok(())
        })
        .map_err(failure)?;
        // Inspect links before deleting their targets, so directory enumeration
        // order cannot leave dangling libtool aliases behind.
        let mut actions = Vec::new();
        for path in candidates {
            let metadata = fs::symlink_metadata(&path)?;
            let link = metadata.file_type().is_symlink();
            if !metadata.is_file() && !link {
                continue;
            }
            let check = if link {
                let target = fs::read_link(&path)?;
                if target.is_absolute() {
                    root.join(target.strip_prefix("/")?)
                } else {
                    path.clone()
                }
            } else {
                path.clone()
            };
            // An unreadable or dangling archive is ignored, as in PortSystem.
            if let Ok(bytes) = fs::read(&check)
                && is_libtool(&bytes)
                && (delete || !link)
            {
                actions.push((path, bytes));
            }
        }
        let mut result = LibtoolCleanup::default();
        for (path, bytes) in actions {
            if delete {
                fs::remove_file(path)?;
                result.removed += 1;
            } else {
                let replacement = clear_dependencies(&bytes);
                if replacement != bytes {
                    rewrite(&path, &replacement)?;
                    result.rewritten += 1;
                }
            }
        }
        Ok(result)
    }
}
fn is_libtool(bytes: &[u8]) -> bool {
    let first = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    first
        .windows(b"a libtool library file".len())
        .any(|window| window == b"a libtool library file")
}
fn clear_dependencies(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line
            .windows(b"dependency_libs".len())
            .any(|window| window == b"dependency_libs")
            && let (Some(first), Some(last)) = (
                line.iter().position(|byte| *byte == b'\''),
                line.iter().rposition(|byte| *byte == b'\''),
            )
            && first < last
        {
            output.extend_from_slice(&line[..=first]);
            output.extend_from_slice(&line[last..]);
            continue;
        }
        output.extend_from_slice(line);
    }
    output
}
fn rewrite(path: &Path, bytes: &[u8]) -> Result<()> {
    // Preserve inode, hardlinks, ownership, and mode. Only regular files are
    // rewritten; refuse a symlink swapped in between inspection and writing.
    let original = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = original.metadata()?;
    ensure!(
        metadata.is_file(),
        "libtool archive is not a regular file: {}",
        path.display()
    );
    let permissions = metadata.permissions();
    original.set_permissions(fs::Permissions::from_mode(permissions.mode() | 0o200))?;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let current = file.metadata()?;
        ensure!(
            current.dev() == metadata.dev() && current.ino() == metadata.ino(),
            "libtool archive changed during cleanup: {}",
            path.display()
        );
        file.set_len(0)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(())
    })();
    let restored = original.set_permissions(permissions);
    result?;
    restored?;
    Ok(())
}

impl Session {
    /// Run the destroot libtool step on this execution-enabled session.
    pub fn clean_destroot_libtool(&mut self) -> Result<LibtoolCleanup> {
        StageContext {
            interp: &mut self.interp,
            id: self.id,
        }
        .clean_destroot_libtool()
    }
}
