//! Native counterpart of PortSystem _get_dep_port and its filesystem probes.
use super::Dependency;
use crate::registry::{Registry, path};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub prefix: PathBuf,
    pub frameworks_dir: PathBuf,
    pub platform: String,
    pub environment: BTreeMap<String, String>,
}
impl Search {
    /// Snapshot the Portfile's current search context, including its environment
    /// array. Missing environment entries stay absent; never substitute the
    /// Rust process environment after a Portfile has changed or unset them.
    pub fn from_session(session: &mut crate::portfile::Session) -> Result<Self> {
        let prefix = session.scalar("prefix")?.into();
        let frameworks_dir = session.scalar("frameworks_dir")?.into();
        let platform = session.scalar("os.platform")?;
        let mut environment = BTreeMap::new();
        for name in [
            "PATH",
            "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_FRAMEWORK_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            let variable = format!("env({name})");
            if session.invoke("info", &["exists", &variable])? == "1" {
                environment.insert(name.into(), session.invoke("set", &[&variable])?);
            }
        }
        Ok(Self {
            prefix,
            frameworks_dir,
            platform,
            environment,
        })
    }
}
pub struct Resolver<'a> {
    registry: Option<&'a Registry>,
    active: Arc<HashSet<String>>,
    search: Search,
}
/// Provider evidence for dependency planning. A registration fallback name is
/// not evidence that its port is installed.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    ActivePort(String),
    System(PathBuf),
    Missing(String),
}
impl Resolution {
    pub fn is_satisfied(&self) -> bool {
        !matches!(self, Self::Missing(_))
    }
}
impl<'a> Resolver<'a> {
    /// Resolve the live runtime dependency lists for registry registration.
    /// Build/test dependencies are excluded; system providers have no registry
    /// dependency entry, and duplicate port names are kept only once.
    pub fn runtime_dependencies(
        &self,
        session: &mut crate::portfile::Session,
    ) -> Result<Vec<String>> {
        let mut dependencies = Vec::new();
        let mut seen = HashSet::new();
        for field in ["depends_lib", "depends_run"] {
            for specification in crate::tcl_list::parse(&session.scalar(field)?)? {
                if let Some(name) = self.resolve(&specification)?
                    && seen.insert(name.to_ascii_lowercase())
                {
                    dependencies.push(name);
                }
            }
        }
        Ok(dependencies)
    }
    /// Cache active port names from the reader's consistent registry snapshot.
    pub fn new(registry: Option<&'a Registry>, search: Search) -> Result<Self> {
        let entries = registry
            .map(Registry::entries)
            .transpose()?
            .unwrap_or_default();
        Ok(Self::from_entries(registry, search, &entries))
    }
    pub(crate) fn from_entries(
        registry: Option<&'a Registry>,
        search: Search,
        entries: &[crate::registry::Entry],
    ) -> Self {
        Self {
            registry,
            search,
            active: Arc::new(
                entries
                    .iter()
                    .filter(|entry| entry.active)
                    .map(|entry| entry.name.to_ascii_lowercase())
                    .collect(),
            ),
        }
    }
    /// Share the immutable registry snapshot while using another Portfile's
    /// search context. Avoid rescanning all installed ports for every dependency.
    pub(crate) fn with_search(&self, search: Search) -> Self {
        Self {
            registry: self.registry,
            active: self.active.clone(),
            search,
        }
    }
    /// Active fallback ports take priority. Otherwise return the owner of the
    /// probed path, None for an unregistered system provider, or the fallback port
    /// when no provider exists. This resolves registration names, not installation.
    pub fn resolve(&self, specification: &str) -> Result<Option<String>> {
        Ok(match self.probe(specification)? {
            Resolution::ActivePort(name) | Resolution::Missing(name) => Some(name),
            Resolution::System(_) => None,
        })
    }
    /// Preserve whether an active port, an unregistered filesystem entry, or no
    /// provider satisfied the dependency. Filesystem evidence is a live snapshot;
    /// callers must recheck it before acting on a long-lived installation plan.
    pub fn probe(&self, specification: &str) -> Result<Resolution> {
        let dependency = Dependency::parse(specification)?;
        if self.active.contains(&dependency.name.to_ascii_lowercase()) {
            return Ok(Resolution::ActivePort(dependency.name));
        }
        let mut parts = specification.split(':');
        let kind = parts.next().unwrap_or_default();
        let pattern = parts.next().unwrap_or_default();
        let (filename, directories, executable) = match kind {
            "bin" => (
                pattern.to_owned(),
                self.search
                    .environment
                    .get("PATH")
                    .context("PATH is required for binary dependency lookup")?
                    .split(':')
                    .map(PathBuf::from)
                    .collect(),
                true,
            ),
            "path" => {
                let full = self.search.prefix.join(pattern);
                let name = full
                    .file_name()
                    .context("dependency path has no filename")?
                    .to_str()
                    .context("dependency filename is not UTF-8")?
                    .to_owned();
                (
                    name,
                    vec![full.parent().unwrap_or(Path::new(".")).to_owned()],
                    false,
                )
            }
            "lib" => {
                let mut directories = Vec::new();
                if let Some(value) = self.search.environment.get("DYLD_FRAMEWORK_PATH") {
                    directories.push(PathBuf::from(value));
                } else {
                    directories.extend([
                        self.search.frameworks_dir.clone(),
                        "/Library/Frameworks".into(),
                        "/Network/Library/Frameworks".into(),
                        "/System/Library/Frameworks".into(),
                    ]);
                }
                for key in ["DYLD_FALLBACK_FRAMEWORK_PATH", "DYLD_LIBRARY_PATH"] {
                    if let Some(value) = self.search.environment.get(key) {
                        directories.push(value.into());
                    }
                }
                directories.extend([
                    "/lib".into(),
                    "/usr/lib".into(),
                    self.search.prefix.join("lib"),
                ]);
                if let Some(value) = self.search.environment.get("DYLD_FALLBACK_LIBRARY_PATH") {
                    directories.push(value.into());
                }
                let split = pattern.find('.').unwrap_or(pattern.len());
                let (name, version) = pattern.split_at(split);
                let filename = if self.search.platform == "darwin" {
                    format!("{name}{version}.dylib")
                } else {
                    format!("{name}.so{version}")
                };
                (filename, directories, false)
            }
            _ => return Ok(Resolution::Missing(dependency.name)),
        };
        for directory in directories {
            if !directory.is_dir() {
                continue;
            }
            let full = directory.join(&filename);
            // Tcl file type uses lstat: non-executable path/lib dependencies
            // may intentionally match directories or dangling symlinks.
            if fs::symlink_metadata(&full).is_err() || (executable && !is_executable(&full)) {
                continue;
            }
            return match self.registry {
                Some(registry) => {
                    let normalized = path::normalize(&full)?;
                    let owner = registry.owner(
                        normalized.to_str().context("provider path is not UTF-8")?,
                        path::case_sensitive(&full),
                    )?;
                    Ok(match owner {
                        Some(name) => Resolution::ActivePort(name),
                        None => Resolution::System(full),
                    })
                }
                None => Ok(Resolution::System(full)),
            };
        }
        Ok(Resolution::Missing(dependency.name))
    }
}
fn is_executable(path: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path remains a valid NUL-terminated string for the call.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}
