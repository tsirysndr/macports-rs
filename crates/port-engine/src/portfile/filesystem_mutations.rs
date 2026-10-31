//! Execution-only filesystem operations for Portfile hooks.
use super::*;
use molt::MoltResult;
use std::{fs, io, path::Path};

pub(super) fn mkdir(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        interp
            .context::<State>(id)
            .unsupported
            .insert("file mkdir".into());
        return Err(error("unsupported Portfile command file mkdir"));
    }
    let mask = interp.context::<State>(id).creation_mask;
    for arg in &argv[2..] {
        let path = arg.as_str();
        let parts = paths::split(path);
        if parts.is_empty() {
            return Err(failure(path, io::Error::from_raw_os_error(libc::ENOENT)));
        }
        // Walk lexical components in order, like Tcl. In particular a/../b
        // creates a before b, and an existing file reports EEXIST at that
        // component rather than ENOTDIR at the complete requested path.
        for end in 1..=parts.len() {
            let target = paths::join(parts[..end].iter().copied());
            match fs::metadata(&target) {
                Ok(metadata) if metadata.is_dir() => (),
                Ok(_) => return Err(failure(&target, io::Error::from_raw_os_error(libc::EEXIST))),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    crate::build::directories::create_all(Path::new(&target), mask)
                        .map_err(|err| failure(&target, err))?;
                }
                Err(err) => return Err(failure(&target, err)),
            }
        }
    }
    Ok(Value::empty())
}

fn failure(path: &str, err: io::Error) -> Exception {
    filesystem::path_error(path, "can't create directory", err)
}
