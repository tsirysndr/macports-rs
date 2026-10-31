//! Deactivate an active image (`portimage::_deactivate_contents`): remove its
//! files, prune directories that become empty, and mark it inactive.
use crate::registry::write::Writer;
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Directories that never belong to a port, even when they become empty.
const PRECIOUS: [&str; 2] = ["/Library/LaunchDaemons", "/Library/LaunchAgents"];

/// Remove the active files of image `id` and record it as inactive. Registry
/// changes commit only after every file is gone; directories are best effort.
pub fn deactivate(writer: &mut Writer, id: i64, files: &[String]) -> Result<()> {
    let mut targets = Vec::new();
    let mut directories = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for file in files {
        let path = Path::new(file);
        if fs::symlink_metadata(path).is_err() {
            continue;
        }
        // Resolve parent symlinks so intermediate links are kept and their
        // empty target directories are removed instead.
        let parent = path.parent().unwrap_or(Path::new("/"));
        let directory = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        targets.push(directory.join(path.file_name().unwrap_or_default()));
        let mut current = Some(directory);
        while let Some(directory) = current {
            if !seen.insert(directory.clone()) {
                break;
            }
            if PRECIOUS
                .iter()
                .any(|precious| directory == Path::new(precious))
            {
                break;
            }
            directories.insert(directory.clone());
            current = directory.parent().map(Path::to_path_buf);
        }
    }
    let mut transaction = writer.transaction()?;
    transaction.deactivate(id)?;
    for target in &targets {
        match fs::remove_file(target) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("cannot remove {}", target.display()));
            }
        }
    }
    transaction.commit()?;
    // Children sort after their parents; remove deepest directories first.
    let ordered: Vec<PathBuf> = directories.into_iter().rev().collect();
    for directory in ordered {
        if directory.parent().is_none() {
            continue;
        }
        if fs::read_dir(&directory).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&directory);
        }
    }
    Ok(())
}
