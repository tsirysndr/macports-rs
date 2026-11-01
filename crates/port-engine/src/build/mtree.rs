//! Native application of the directory-only mtree format used by MacPorts.
//! Ownership is applied only when running as root, as `mtree -U` can.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    mode: u32,
    attrs: BTreeMap<String, String>,
}

/// A validated directory specification. Parsing completes before any mutation.
#[derive(Debug)]
pub struct DirectorySpec {
    entries: Vec<Entry>,
}
impl DirectorySpec {
    pub fn parse(source: &str) -> Result<Self> {
        let mut defaults = BTreeMap::<String, String>::new();
        let mut entries = Vec::new();
        let mut current: Option<PathBuf> = None;
        for (number, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let name = words.next().unwrap();
            let context = || format!("mtree line {}", number + 1);
            if name == "/set" {
                attributes(words, &mut defaults).with_context(context)?;
                continue;
            }
            if name == "/unset" {
                for key in words {
                    if key == "all" {
                        defaults.clear();
                    } else {
                        ensure!(
                            matches!(key, "type" | "mode" | "uname" | "gname" | "uid" | "gid"),
                            "unsupported mtree attribute {key}"
                        );
                        defaults.remove(key);
                    }
                }
                continue;
            }
            if name == ".." {
                ensure!(
                    words.next().is_none(),
                    "unexpected attributes on mtree parent"
                );
                let path = current.as_mut().context("mtree parent without directory")?;
                if !path.pop() {
                    current = None;
                }
                continue;
            }
            ensure!(!name.starts_with('/'), "unsupported mtree directive {name}");
            let path = if name == "." {
                ensure!(current.is_none(), "nested mtree root");
                PathBuf::new()
            } else {
                ensure!(
                    !name.contains(['/', '\\', '\0']) && !name.contains(['*', '?', '[']),
                    "unsupported mtree directory name {name}"
                );
                current
                    .as_ref()
                    .context("mtree entry before root")?
                    .join(name)
            };
            let mut attrs = defaults.clone();
            attributes(words, &mut attrs).with_context(context)?;
            ensure!(
                attrs.get("type").is_some_and(|kind| kind == "dir"),
                "mtree entry must have type=dir"
            );
            let raw = attrs.get("mode").context("mtree directory has no mode")?;
            let mode = u32::from_str_radix(raw, 8).context("invalid mtree octal mode")?;
            ensure!(mode <= 0o7777, "invalid mtree mode {raw}");
            current = Some(path.clone());
            entries.push(Entry { path, mode, attrs });
        }
        ensure!(!entries.is_empty(), "empty mtree directory specification");
        Ok(Self { entries })
    }

    /// Apply directory modes as `mtree -e -U` does, leaving extra entries intact.
    /// Non-root staging retains caller ownership. Physical directories are required;
    /// links are rejected rather than followed. The caller must own the stage and
    /// prevent concurrent filesystem changes while this operation runs.
    pub fn apply_unprivileged(&self, root: &Path) -> Result<()> {
        ensure!(
            unsafe { libc::getuid() != 0 && libc::geteuid() != 0 },
            "privileged mtree ownership requires the installation worker"
        );
        self.apply_modes(root)
    }

    /// `mtree -e -U`: as root, also give each directory its specified owner
    /// and group; otherwise the caller keeps ownership.
    pub fn apply(&self, root: &Path) -> Result<()> {
        self.apply_modes(root)?;
        // SAFETY: identity queries have no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return Ok(());
        }
        for entry in &self.entries {
            let (uid, gid) = owner(&entry.attrs)?;
            if uid.is_none() && gid.is_none() {
                continue;
            }
            let path = root.join(&entry.path);
            std::os::unix::fs::lchown(&path, uid, gid)
                .with_context(|| format!("cannot set owner of {}", path.display()))?;
        }
        Ok(())
    }

    fn apply_modes(&self, root: &Path) -> Result<()> {
        ensure!(
            fs::symlink_metadata(root)?.file_type().is_dir(),
            "mtree root must be a physical directory"
        );
        // Preflight existing entries before changing permissions or creating paths.
        for entry in &self.entries {
            let mut path = root.to_owned();
            for component in entry.path.components() {
                path.push(component);
                match fs::symlink_metadata(&path) {
                    Ok(meta) => ensure!(
                        meta.file_type().is_dir(),
                        "mtree entry is not a physical directory: {}",
                        path.display()
                    ),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
                    Err(err) => return Err(err.into()),
                }
            }
        }
        // Apply modes after descendants, so restrictive parent modes do not
        // prevent their children from being created.
        for entry in &self.entries {
            let path = root.join(&entry.path);
            if !path.exists() {
                super::directories::create_all(&path, Some(0o077))?;
            }
        }
        let mut final_modes = BTreeMap::new();
        for entry in &self.entries {
            final_modes.insert(&entry.path, entry.mode);
        }
        let mut final_modes: Vec<_> = final_modes.into_iter().collect();
        final_modes.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
        for (path, mode) in final_modes {
            fs::set_permissions(root.join(path), fs::Permissions::from_mode(mode))?;
        }
        Ok(())
    }
}
/// The ids named by `uid`/`uname` and `gid`/`gname`, numeric ids first.
fn owner(attrs: &BTreeMap<String, String>) -> Result<(Option<u32>, Option<u32>)> {
    use std::ffi::CString;
    let uid = match (attrs.get("uid"), attrs.get("uname")) {
        (Some(uid), _) => Some(uid.parse().context("invalid mtree uid")?),
        (None, Some(name)) => {
            let name = CString::new(name.as_str())?;
            // SAFETY: the name is NUL terminated; the record is read immediately.
            let record = unsafe { libc::getpwnam(name.as_ptr()) };
            ensure!(
                !record.is_null(),
                "unknown mtree user {}",
                name.to_string_lossy()
            );
            Some(unsafe { (*record).pw_uid })
        }
        (None, None) => None,
    };
    let gid = match (attrs.get("gid"), attrs.get("gname")) {
        (Some(gid), _) => Some(gid.parse().context("invalid mtree gid")?),
        (None, Some(name)) => {
            let name = CString::new(name.as_str())?;
            // SAFETY: the name is NUL terminated; the record is read immediately.
            let record = unsafe { libc::getgrnam(name.as_ptr()) };
            ensure!(
                !record.is_null(),
                "unknown mtree group {}",
                name.to_string_lossy()
            );
            Some(unsafe { (*record).gr_gid })
        }
        (None, None) => None,
    };
    Ok((uid, gid))
}
fn attributes<'a>(
    words: impl Iterator<Item = &'a str>,
    attrs: &mut BTreeMap<String, String>,
) -> Result<()> {
    for word in words {
        let Some((key, value)) = word.split_once('=') else {
            bail!("invalid mtree attribute {word}");
        };
        ensure!(
            matches!(key, "type" | "mode" | "uname" | "gname" | "uid" | "gid"),
            "unsupported mtree attribute {key}"
        );
        ensure!(!value.is_empty(), "empty mtree attribute {key}");
        attrs.insert(key.into(), value.into());
    }
    Ok(())
}
