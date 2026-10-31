//! Tcl file delete for execution-enabled Portfile hooks.
use super::*;
use molt::MoltResult;
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        interp
            .context::<State>(id)
            .unsupported
            .insert("file delete".into());
        return Err(error("unsupported Portfile command file delete"));
    }
    let mut start = 2;
    let mut force = false;
    while let Some(arg) = argv.get(start) {
        match arg.as_str() {
            "-force" => force = true,
            "--" => {
                start += 1;
                break;
            }
            option if option.starts_with('-') => {
                return Err(error(format!(
                    "bad option \"{option}\": must be -force or --"
                )));
            }
            _ => break,
        }
        start += 1;
    }
    for arg in &argv[start..] {
        let path = paths::join([arg.as_str()]);
        if let Err((failed, err)) = remove(Path::new(&path), force) {
            // Tcl ignores ENOENT even if another process removed an entry
            // between inspection and unlinking.
            if err.kind() == io::ErrorKind::NotFound {
                continue;
            }
            let failed = if failed == Path::new(&path) {
                arg.as_str().to_owned()
            } else {
                failed.to_string_lossy().into_owned()
            };
            if matches!(err.raw_os_error(), Some(libc::ENOTEMPTY | libc::EEXIST)) {
                if force {
                    return Err(filesystem::path_error(
                        &failed,
                        "error deleting",
                        io::Error::from_raw_os_error(libc::EEXIST),
                    ));
                }
                return Err(Exception::molt_err2(
                    Value::from(vec![
                        "POSIX".into(),
                        "EEXIST".into(),
                        "file already exists".into(),
                    ]),
                    format!("error deleting \"{failed}\": directory not empty").into(),
                ));
            }
            return Err(filesystem::path_error(&failed, "error deleting", err));
        }
    }
    Ok(Value::empty())
}

type RemovalResult = std::result::Result<(), (PathBuf, io::Error)>;
fn at(path: &Path, err: io::Error) -> (PathBuf, io::Error) {
    (path.to_owned(), err)
}
fn remove(path: &Path, force: bool) -> RemovalResult {
    let metadata = fs::symlink_metadata(path).map_err(|err| at(path, err))?;
    if !metadata.is_dir() {
        return fs::remove_file(path).map_err(|err| at(path, err));
    }
    if !force {
        return fs::remove_dir(path).map_err(|err| at(path, err));
    }
    // Upstream grants owner access to the root directory for -force, and
    // restores its original permissions on failure. Descendants keep theirs.
    let permissions = metadata.permissions();
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(permissions.mode() | 0o700));
    let result = match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if matches!(err.raw_os_error(), Some(libc::ENOTEMPTY | libc::EEXIST)) => {
            remove_tree(path)
        }
        Err(err) => Err(at(path, err)),
    };
    if result.is_err() {
        let _ = fs::set_permissions(path, permissions);
    }
    result
}

fn remove_tree(root: &Path) -> RemovalResult {
    // Iterative postorder walk avoids recursion limits. symlink_metadata makes
    // directory links leaf entries, including dangling links and link cycles.
    let mut pending = vec![(root.to_owned(), false)];
    while let Some((path, visited)) = pending.pop() {
        if visited {
            fs::remove_dir(&path).map_err(|err| at(&path, err))?;
        } else {
            let metadata = fs::symlink_metadata(&path).map_err(|err| at(&path, err))?;
            if metadata.is_dir() {
                let entries = fs::read_dir(&path).map_err(|err| at(&path, err))?;
                pending.push((path.clone(), true));
                for entry in entries {
                    pending.push((entry.map_err(|err| at(&path, err))?.path(), false));
                }
            } else {
                fs::remove_file(&path).map_err(|err| at(&path, err))?;
            }
        }
    }
    Ok(())
}
