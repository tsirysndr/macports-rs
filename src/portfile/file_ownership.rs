//! Native owner/group file attributes, following symlinks like Tcl.
use super::*;
use molt::MoltResult;
use std::{ffi::CString, fs, io, os::unix::fs::MetadataExt};

pub(super) fn attribute(path: &str, group: bool, value: Option<&Value>) -> MoltResult {
    let kind = if group { "group" } else { "owner" };
    let action = format!("could not set {kind} for file");
    let Some(value) = value else {
        let metadata =
            fs::metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
        let id = if group {
            metadata.gid()
        } else {
            metadata.uid()
        };
        let name = accounts::by_id(id, group)
            .ok()
            .flatten()
            .map(|account| account.name)
            .unwrap_or_else(|| id.to_string());
        return Ok(name.into());
    };
    let id = if let Ok(id) = value.as_int() {
        id as u32
    } else {
        accounts::by_name(value.as_str(), group)
            .ok()
            .flatten()
            .map(|account| account.id)
            .ok_or_else(|| {
                let (operation, code, noun) = if group {
                    ("SETGRP", "NO_GROUP", "group")
                } else {
                    ("SETOWN", "NO_USER", "user")
                };
                Exception::molt_err2(
                    Value::from(vec![
                        "TCL".into(),
                        "OPERATION".into(),
                        operation.into(),
                        code.into(),
                    ]),
                    format!("{action} \"{path}\": {noun} \"{value}\" does not exist").into(),
                )
            })?
    };
    let native = CString::new(path).map_err(|_| {
        filesystem::path_error(path, &action, io::Error::from_raw_os_error(libc::EINVAL))
    })?;
    let (uid, gid) = if group {
        (u32::MAX, id)
    } else {
        (id, u32::MAX)
    };
    // SAFETY: native is NUL terminated and alive for the call. The all-ones
    // sentinel preserves the other identity; chown follows the final symlink.
    if unsafe { libc::chown(native.as_ptr(), uid, gid) } != 0 {
        return Err(filesystem::path_error(
            path,
            &action,
            io::Error::last_os_error(),
        ));
    }
    Ok(Value::empty())
}
