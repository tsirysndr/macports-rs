//! Unix Tcl path normalization, including nonexistent suffixes and final links.
use super::*;
use molt::{MoltResult, check_args};
use std::{fs, io};

pub(super) fn command(argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 3, "name")?;
    let path = argv[2].as_str();
    if path.is_empty() {
        return Ok(Value::empty());
    }
    if path.contains('\0') {
        return Err(filesystem::file_error(
            path,
            false,
            io::Error::from_raw_os_error(libc::EINVAL),
        ));
    }
    let absolute = if path.starts_with('/') {
        paths::join([path])
    } else {
        let cwd = fs::canonicalize(".")
            .map_err(|err| filesystem::path_error(".", "could not get working directory", err))?;
        let cwd = cwd
            .to_str()
            .ok_or_else(|| error("working directory is not valid UTF-8"))?;
        paths::join([cwd, path])
    };
    let input = paths::split(&absolute);
    let mut parts = vec![input[0].to_owned()];
    for part in &input[1..] {
        match *part {
            "." => (),
            ".." => {
                // Tcl checks whether the preceding path is itself a link
                // before backing up. read_link resolves intermediate links
                // and leaves a dangling final target available for this step.
                let current = paths::join(parts.iter().map(String::as_str));
                if let Ok(target) = fs::read_link(&current) {
                    let target = target
                        .to_str()
                        .ok_or_else(|| error("link target is not valid UTF-8"))?;
                    parts.pop();
                    let parent = paths::join(parts.iter().map(String::as_str));
                    let expanded = paths::join([parent.as_str(), target]);
                    parts = paths::split(&expanded)
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                }
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            value => parts.push(value.to_owned()),
        }
    }
    // Resolve the longest existing parent, never the final component. Tcl
    // tolerates missing/inaccessible suffixes instead of requiring realpath
    // to succeed for the entire requested path.
    for count in (1..parts.len()).rev() {
        let prefix = paths::join(parts[..count].iter().map(String::as_str));
        if let Ok(canonical) = fs::canonicalize(&prefix) {
            let canonical = canonical
                .to_str()
                .ok_or_else(|| error("normalized path is not valid UTF-8"))?;
            return Ok(paths::join(
                std::iter::once(canonical).chain(parts[count..].iter().map(String::as_str)),
            )
            .into());
        }
    }
    Ok(paths::join(parts.iter().map(String::as_str)).into())
}
