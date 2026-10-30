//! Keepdir markers, empty-directory pruning, and the empty-stage guard.
use super::*;
use crate::progress::Progress;
use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DestrootDirectories {
    pub created_markers: usize,
    pub removed_directories: usize,
}
impl Session {
    /// Apply the keepdirs/pruning portion of destroot_finish after file cleanup.
    /// This does not perform ownership, compression, or target-state updates.
    pub fn prune_destroot_directories(&mut self) -> Result<DestrootDirectories> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "destroot pruning requires an execution-enabled session"
        );
        let root = PathBuf::from(self.scalar("destroot")?);
        let subport = self.scalar("subport")?;
        ensure!(
            !subport.is_empty() && !subport.contains('/') && !subport.contains('\0'),
            "invalid subport name for keepdir marker"
        );
        let keeps = self
            .interp
            .scalar("destroot.keepdirs")
            .map_err(failure)?
            .as_list()
            .map_err(failure)?
            .to_vec();
        let canonical = fs::canonicalize(&root)?;
        let mut result = DestrootDirectories::default();
        for keep in keeps {
            let directory = jailed(&canonical, Path::new(keep.as_str()))?;
            crate::build::directories::create_all(&directory, Some(0o022))?;
            let marker = directory.join(format!(".turd_{subport}"));
            if marker.exists() {
                continue;
            }
            // A dangling marker link must be replaced, never followed outside
            // the stage while installing the empty marker.
            if fs::symlink_metadata(&marker).is_ok_and(|meta| meta.file_type().is_symlink()) {
                fs::remove_file(&marker)?;
            }
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker)
            {
                Ok(file) => {
                    file.set_permissions(fs::Permissions::from_mode(0o644))?;
                    result.created_markers += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists && marker.exists() => (),
                Err(err) => return Err(err.into()),
            }
        }
        traverse::visit_paths(vec![root.clone()], |path| {
            let metadata = fs::symlink_metadata(path)
                .map_err(|err| filesystem::file_error(&path.to_string_lossy(), true, err))?;
            // Only rmdir, never recursive removal. Failures (usually a nonempty
            // directory) are intentionally ignored as in PortSystem's catch.
            if metadata.is_dir() && fs::remove_dir(path).is_ok() {
                result.removed_directories += 1;
            }
            Ok(())
        })
        .map_err(failure)?;
        if !root.is_dir() {
            let progress = self
                .interp
                .context::<State>(self.id)
                .progress
                .clone()
                .unwrap_or_else(|| Progress::new(false));
            progress.message("Error: No files have been installed in the destroot directory!");
            progress.message("Error: Please make sure this software supports 'make install DESTDIR=${destroot}' or implement an alternative destroot mechanism in the Portfile.");
            progress.message("Error: Files might have been installed directly into your system, check before proceeding.");
            anyhow::bail!("Staging {subport} into destroot failed");
        }
        Ok(result)
    }
}
fn normalized(path: &Path) -> Result<PathBuf> {
    // The dummy component makes native Tcl normalization resolve a final link
    // while retaining support for directories which have not been created yet.
    let dummy = path.join("__port_rs_keepdir__");
    let text = dummy
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("keepdir path is not valid UTF-8"))?;
    let value = file_normalize::command(&["file".into(), "normalize".into(), text.into()])
        .map_err(failure)?;
    Ok(Path::new(value.as_str())
        .parent()
        .unwrap_or(Path::new("/"))
        .to_owned())
}
fn jailed(root: &Path, input: &Path) -> Result<PathBuf> {
    if input.is_absolute() {
        let resolved = normalized(input)?;
        if resolved.starts_with(root) {
            return Ok(resolved);
        }
    }
    // Strip an external absolute root and collapse relative modifiers before
    // prepending destroot. Parent references cannot escape the staging root.
    let mut relative = PathBuf::new();
    for part in input.components() {
        match part {
            Component::Normal(name) => relative.push(name),
            Component::ParentDir => {
                relative.pop();
            }
            _ => (),
        }
    }
    let path = normalized(&root.join(relative))?;
    ensure!(
        path.starts_with(root),
        "keepdir resolves outside destroot: {}",
        input.display()
    );
    Ok(path)
}
