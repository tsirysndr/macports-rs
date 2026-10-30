//! Host-side build-state opening and Portfile-change invalidation.
use super::state::{Compatibility, StateFile};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::MetadataExt, path::Path, time::SystemTime};

pub struct PreparedState {
    pub state: StateFile,
    /// The compatibility observed before any stale artifacts were removed.
    pub compatibility: Compatibility,
    pub initialized: bool,
}

/// Open state for a real build after work-directory migration and privilege
/// setup. This is not a dry-run API. The caller owns the work directory.
///
/// Unlike upstream's delete-before-lock sequence, cleanup retains both the
/// work directory and the state inode. Cooperating builders therefore wait on
/// the same lock throughout invalidation. External tools deleting that inode
/// are not protected by this advisory lock.
pub fn prepare(
    work: &Path,
    subport: &str,
    portfile: &Path,
    creation_mask: Option<u32>,
    ignore_different: bool,
) -> Result<PreparedState> {
    ensure!(
        !subport.is_empty() && !subport.contains('/') && !subport.contains('\0'),
        "invalid subport name for build state"
    );
    let source = fs::read(portfile)
        .with_context(|| format!("cannot read Portfile {}", portfile.display()))?;
    let checksum = format!("{:x}", Sha256::digest(&source));
    let modified = fs::metadata(portfile)?.mtime();
    let path = work.join(format!(".macports.{subport}.state"));
    // Capture this before opening: fresh builds allow future Portfiles, as in
    // upstream, whereas an existing build rejects them even with -o.
    if path.exists() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();
        ensure!(
            i128::from(modified) <= i128::from(now),
            "Portfile for {subport} is from the future - check date and time of your system"
        );
    }
    let mut state = StateFile::open(&path, creation_mask)?;
    // Recheck records after taking the lock: another builder may have finished
    // initializing or invalidating while this caller waited.
    let fresh = state.lines()?.is_empty();
    let compatibility = if fresh || ignore_different {
        Compatibility::Current
    } else {
        state.compatibility(&checksum, modified)?
    };
    let changed = compatibility == Compatibility::PortfileChanged;
    if changed {
        for entry in fs::read_dir(work)? {
            let entry = entry?;
            if entry.path() == path {
                continue;
            }
            // file_type does not follow symlinks, including links to directories.
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        // Only declare fresh state after all cleanup has succeeded.
        state.reset(&checksum)?;
    } else if fresh {
        state.initialize(&checksum)?;
    }
    Ok(PreparedState {
        state,
        compatibility,
        initialized: fresh || changed,
    })
}
