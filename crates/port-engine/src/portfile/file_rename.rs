//! Native same-filesystem renames for Portfile hooks and work-directory migration.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        interp
            .context::<State>(id)
            .unsupported
            .insert("file rename".into());
        return Err(error("unsupported Portfile command file rename"));
    }
    let (mut index, mut force) = (2, false);
    while let Some(arg) = argv.get(index) {
        match arg.as_str() {
            "-force" => force = true,
            "--" => {
                index += 1;
                break;
            }
            value if value.starts_with('-') => {
                return Err(error(format!(
                    "bad option \"{value}\": must be -force or --"
                )));
            }
            _ => break,
        }
        index += 1;
    }
    check_args(
        2,
        argv,
        index + 2,
        0,
        "?-option value ...? source ?source ...? target",
    )?;
    let target = argv.last().unwrap().as_str();
    let directory = fs::metadata(target).is_ok_and(|metadata| metadata.is_dir());
    if !directory && argv.len() - index > 2 {
        return Err(custom_error(
            libc::ENOTDIR,
            format!("error renaming: target \"{target}\" is not a directory"),
        ));
    }
    for source in &argv[index..argv.len() - 1] {
        let source = source.as_str();
        let destination = if directory {
            let parts = paths::split(source);
            let tail = if parts.len() == 1 && source.starts_with('/') {
                ""
            } else {
                parts.last().copied().unwrap_or("")
            };
            paths::join([target, tail])
        } else {
            target.to_owned()
        };
        rename_one(source, &destination, force)?;
    }
    Ok(Value::empty())
}
fn rename_one(source: &str, target: &str, force: bool) -> MoltResult {
    let src = paths::join([source]);
    let dst = paths::join([target]);
    let source_metadata = fs::symlink_metadata(&src)
        .map_err(|err| filesystem::path_error(source, "error renaming", err))?;
    match fs::symlink_metadata(&dst) {
        Ok(target_metadata) => {
            if !force {
                return Err(failure(
                    source,
                    target,
                    io::Error::from_raw_os_error(libc::EEXIST),
                ));
            }
            if source_metadata.ino() != 0
                && source_metadata.ino() == target_metadata.ino()
                && source_metadata.dev() == target_metadata.dev()
            {
                return Ok(Value::empty());
            }
            if source_metadata.is_dir() != target_metadata.is_dir() {
                let (old, new) = if source_metadata.is_dir() {
                    ("file", "directory")
                } else {
                    ("directory", "file")
                };
                return Err(Exception::molt_err(
                    format!("can't overwrite {old} \"{target}\" with {new} \"{source}\"").into(),
                ));
            }
            // Match Tcl's best-effort owner-write adjustment before replacement.
            if let Ok(metadata) = fs::metadata(&dst) {
                let _ =
                    fs::set_permissions(&dst, fs::Permissions::from_mode(metadata.mode() | 0o200));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => return Err(failure(source, target, err)),
    }
    fs::rename(&src, &dst).map_err(|err| {
        match err.raw_os_error() {
            Some(libc::EINVAL) => Exception::molt_err(format!("error renaming \"{source}\" to \"{target}\": trying to rename a volume or move a directory into itself").into()),
            Some(libc::ENOTEMPTY) => failure(source, target, io::Error::from_raw_os_error(libc::EEXIST)),
            // Cross-volume copy/delete fallback is not implemented yet. Keep
            // the source intact and return EXDEV instead of pretending success.
            _ => failure(source, target, err),
        }
    })?;
    Ok(Value::empty())
}
fn failure(source: &str, target: &str, err: io::Error) -> Exception {
    filesystem::path_error(&format!("{source}\" to \"{target}"), "error renaming", err)
}
fn custom_error(errno: i32, message: String) -> Exception {
    let base = filesystem::file_error("", false, io::Error::from_raw_os_error(errno));
    Exception::molt_err2(base.error_code(), message.into())
}
