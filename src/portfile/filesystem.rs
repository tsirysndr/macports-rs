//! Read-only native Unix file queries used by compiler and PortGroup setup.
use super::error;
use molt::{Exception, Interp, MoltResult, Value, check_args};
use std::{
    ffi::CString,
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt},
};

pub(super) fn is_metadata_query(operation: &str) -> bool {
    matches!(
        operation,
        "size" | "mtime" | "atime" | "type" | "readlink" | "stat" | "lstat"
    )
}

pub(super) fn metadata_query(interp: &mut Interp, argv: &[Value]) -> MoltResult {
    let operation = argv[1].as_str();
    let stat = matches!(operation, "stat" | "lstat");
    check_args(
        2,
        argv,
        3,
        if stat { 4 } else { 3 },
        if stat { "name ?varName?" } else { "name" },
    )?;
    let path = argv[2].as_str();
    if operation == "readlink" {
        let target = fs::read_link(path).map_err(|err| file_error(path, true, err))?;
        return target
            .into_os_string()
            .into_string()
            .map(Value::from)
            .map_err(|_| error(format!("link target for \"{path}\" is not valid UTF-8")));
    }
    let metadata = if matches!(operation, "type" | "lstat") {
        fs::symlink_metadata(path)
    } else {
        fs::metadata(path)
    }
    .map_err(|err| file_error(path, false, err))?;
    match operation {
        "size" => Ok(metadata.size().to_string().into()),
        "mtime" => Ok(metadata.mtime().into()),
        "atime" => Ok(metadata.atime().into()),
        "type" => Ok(file_type(&metadata).into()),
        "stat" | "lstat" => {
            let mut fields: Vec<(&str, Value)> = vec![
                ("dev", metadata.dev().to_string().into()),
                ("ino", metadata.ino().to_string().into()),
                ("nlink", metadata.nlink().to_string().into()),
                ("uid", metadata.uid().to_string().into()),
                ("gid", metadata.gid().to_string().into()),
                ("size", metadata.size().to_string().into()),
                ("blocks", metadata.blocks().to_string().into()),
                ("blksize", metadata.blksize().to_string().into()),
            ];
            // Tcl exposes rdev only in the array form, and only for devices.
            if argv.len() == 4
                && (metadata.file_type().is_char_device() || metadata.file_type().is_block_device())
            {
                fields.push(("rdev", metadata.rdev().to_string().into()));
            }
            fields.extend([
                ("atime", metadata.atime().into()),
                ("mtime", metadata.mtime().into()),
                ("ctime", metadata.ctime().into()),
                ("mode", (metadata.mode() as u16).to_string().into()),
                ("type", file_type(&metadata).into()),
            ]);
            if argv.len() == 4 {
                for (key, value) in fields {
                    interp.set_element(argv[3].as_str(), key, value)?;
                }
                Ok(Value::empty())
            } else {
                Ok(Value::from(
                    fields
                        .into_iter()
                        .flat_map(|(key, value)| [key.into(), value])
                        .collect::<Vec<Value>>(),
                ))
            }
        }
        _ => unreachable!(),
    }
}

fn file_type(metadata: &fs::Metadata) -> &'static str {
    let kind = metadata.file_type();
    if kind.is_file() {
        "file"
    } else if kind.is_dir() {
        "directory"
    } else if kind.is_char_device() {
        "characterSpecial"
    } else if kind.is_block_device() {
        "blockSpecial"
    } else if kind.is_fifo() {
        "fifo"
    } else if kind.is_symlink() {
        "link"
    } else if kind.is_socket() {
        "socket"
    } else {
        "unknown"
    }
}

fn file_error(path: &str, link: bool, err: io::Error) -> Exception {
    // Tcl's POSIX error code contains the symbolic errno and its stable English
    // description, rather than Rust's platform-dependent '(os error N)' suffix.
    let (name, description) = match err.raw_os_error() {
        Some(libc::ENOENT) => ("ENOENT", "no such file or directory"),
        Some(libc::EACCES) => ("EACCES", "permission denied"),
        Some(libc::EPERM) => ("EPERM", "operation not permitted"),
        Some(libc::ENOTDIR) => ("ENOTDIR", "not a directory"),
        Some(libc::ELOOP) => ("ELOOP", "too many levels of symbolic links"),
        Some(libc::ENAMETOOLONG) => ("ENAMETOOLONG", "file name too long"),
        Some(libc::EIO) => ("EIO", "input/output error"),
        Some(libc::ENOMEM) => ("ENOMEM", "cannot allocate memory"),
        Some(libc::EINTR) => ("EINTR", "interrupted system call"),
        Some(libc::EOVERFLOW) => ("EOVERFLOW", "value too large for defined data type"),
        Some(libc::ESTALE) => ("ESTALE", "stale file handle"),
        Some(libc::EINVAL) => ("EINVAL", "invalid argument"),
        _ if err.kind() == io::ErrorKind::InvalidInput => ("EINVAL", "invalid argument"),
        _ => ("UNKNOWN", "unknown error"),
    };
    let action = if link { "read link" } else { "read" };
    Exception::molt_err2(
        Value::from(vec!["POSIX".into(), name.into(), description.into()]),
        format!("could not {action} \"{path}\": {description}").into(),
    )
}

pub(super) fn is_predicate(operation: &str) -> bool {
    matches!(
        operation,
        "exists" | "isfile" | "isdirectory" | "readable" | "writable" | "executable" | "owned"
    )
}

pub(super) fn predicate(operation: &str, path: &str) -> bool {
    match operation {
        "exists" => access(path, libc::F_OK),
        "readable" => access(path, libc::R_OK),
        "writable" => access(path, libc::W_OK),
        "executable" => access(path, libc::X_OK),
        // Tcl's stat predicates follow symlinks and return false on all errors,
        // including dangling links, inaccessible parents, and invalid paths.
        "isfile" => fs::metadata(path).is_ok_and(|metadata| metadata.is_file()),
        "isdirectory" => fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()),
        "owned" => fs::metadata(path).is_ok_and(|metadata| {
            // SAFETY: geteuid takes no arguments and has no memory preconditions.
            metadata.uid() == unsafe { libc::geteuid() }
        }),
        _ => unreachable!("file predicate dispatch must validate the operation"),
    }
}

fn access(path: &str, mode: libc::c_int) -> bool {
    let Ok(path) = CString::new(path) else {
        return false;
    };
    // Match Tcl's Unix access(), which checks real IDs and honors filesystem
    // permissions/ACLs. Testing mode bits alone gives incorrect answers.
    // SAFETY: path is a live NUL-terminated string; mode is an access constant.
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}
