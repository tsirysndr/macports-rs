//! Whole-second file timestamps following Tcl's stat/utime/stat semantics.
use super::*;
use molt::{MoltResult, check_args};
use std::{ffi::CString, fs, io, os::unix::fs::MetadataExt};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 4, "name ?time?")?;
    if argv.len() == 3 {
        return filesystem::metadata_query(interp, argv);
    }
    let operation = argv[1].as_str();
    if interp.context::<State>(id).system_executor.is_none() {
        let name = format!("file {operation} setter");
        interp.context::<State>(id).unsupported.insert(name.clone());
        return Err(error(format!("unsupported Portfile command {name}")));
    }
    let path = argv[2].as_str();
    // Upstream checks the file before parsing the new timestamp. Follow links,
    // preserve the other field's seconds, and discard subsecond precision in
    // both fields, as Tcl's utimbuf interface does.
    let metadata = fs::metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
    let seconds = argv[3].as_int()?;
    let access = operation == "atime";
    let times = [
        libc::timeval {
            tv_sec: if access { seconds } else { metadata.atime() } as libc::time_t,
            tv_usec: 0,
        },
        libc::timeval {
            tv_sec: if access { metadata.mtime() } else { seconds } as libc::time_t,
            tv_usec: 0,
        },
    ];
    let native = CString::new(path).map_err(|_| {
        filesystem::file_error(path, false, io::Error::from_raw_os_error(libc::EINVAL))
    })?;
    // SAFETY: native is NUL terminated and times contains the two timeval
    // records required by utimes. Both remain alive throughout the syscall.
    if unsafe { libc::utimes(native.as_ptr(), times.as_ptr()) } != 0 {
        return Err(filesystem::path_error(
            path,
            if access {
                "could not set access time for file"
            } else {
                "could not set modification time for file"
            },
            io::Error::last_os_error(),
        ));
    }
    // Filesystems may round or constrain timestamps; return the stored value.
    let metadata = fs::metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
    Ok(if access {
        metadata.atime()
    } else {
        metadata.mtime()
    }
    .into())
}
