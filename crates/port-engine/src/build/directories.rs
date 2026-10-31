//! Directory preparation with an explicit mask and no process-global umask changes.
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::Path,
};

pub fn create_all(path: &Path, mask: Option<u32>) -> io::Result<()> {
    let Some(mask) = mask else {
        return fs::create_dir_all(path);
    };
    if path.try_exists()? && !path.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EEXIST));
    }
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.as_os_str().is_empty() && !parent.try_exists()? {
        missing.push(parent);
        let Some(next) = parent.parent() else {
            break;
        };
        parent = next;
    }
    for path in missing.into_iter().rev() {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            // chmod only directories this operation created. Starting private
            // avoids exposing broader permissions before applying the session mask.
            Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o777 & !mask))?,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => (),
            Err(err) => return Err(err),
        }
    }
    Ok(())
}
