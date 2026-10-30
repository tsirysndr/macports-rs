//! Normalize parent directories while preserving a final symlink's identity.
use anyhow::{Context, Result, ensure};
use std::path::{Component, Path, PathBuf};

pub fn normalize(path: &Path) -> Result<PathBuf> {
    let expanded;
    let path = if path.starts_with("~") {
        expanded = PathBuf::from(std::env::var_os("HOME").context("HOME is required to expand ~")?)
            .join(path.strip_prefix("~").unwrap());
        expanded.as_path()
    } else {
        path
    };
    ensure!(!path.as_os_str().is_empty(), "empty filename");
    let absolute = std::path::absolute(path)?;
    let components: Vec<_> = absolute.components().collect();
    let mut normalized = PathBuf::new();
    for (i, component) in components.iter().enumerate() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            _ => {
                normalized.push(component.as_os_str());
                if i + 1 < components.len() {
                    match normalized.canonicalize() {
                        Ok(parent) => normalized = parent,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error).context("cannot normalize parent directory");
                        }
                    }
                }
            }
        }
    }
    Ok(normalized)
}

#[cfg(target_os = "macos")]
pub fn case_sensitive(path: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: the path is a valid NUL-terminated string. Like upstream, a failed
    // capability query falls back to a case-insensitive registry lookup.
    unsafe { libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE) > 0 }
}
#[cfg(not(target_os = "macos"))]
pub fn case_sensitive(_path: &Path) -> bool {
    true
}
