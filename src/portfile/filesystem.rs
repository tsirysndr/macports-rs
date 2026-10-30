//! Read-only native Unix file predicates used by compiler and PortGroup setup.
use std::{ffi::CString, fs, os::unix::fs::MetadataExt};

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
