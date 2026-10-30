//! MacPorts startup environment selection, before Portfile evaluation.
use std::{
    collections::{BTreeMap, HashSet},
    ffi::CString,
    os::unix::fs::MetadataExt,
};

/// Configuration and resolved identities supplied by the startup driver. This
/// does not change credentials or mutate the Rust process's environment.
pub struct Startup<'a> {
    pub prefix: &'a str,
    pub binpath: Option<&'a str>,
    pub extra_env: &'a [String],
    /// Real invoking uid, corresponding to upstream getuid (not effective uid).
    pub invoking_uid: u32,
    /// Resolved uid of the configured macportsuser.
    pub macports_uid: u32,
}
impl Startup<'_> {
    /// Filter explicit inherited values using macports::mportinit's allowlist.
    /// Proxy configuration overrides and per-build HOME/TMPDIR preparation occur
    /// later. extra_env intentionally permits even otherwise rejected TMP values.
    pub fn prepare(&self, inherited: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut keep: HashSet<&str> = [
            "DISPLAY",
            "DYLD_FALLBACK_FRAMEWORK_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
            "DYLD_FRAMEWORK_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "HOME",
            "JAVA_HOME",
            "MASTER_SITE_LOCAL",
            "ARCHIVE_SITE_LOCAL",
            "PATCH_SITE_LOCAL",
            "PATH",
            "PORTSRC",
            "RSYNC_PROXY",
            "USER",
            "GROUP",
            "LANG",
            "http_proxy",
            "COLUMNS",
            "LINES",
            "https_proxy",
            "HTTPS_PROXY",
            "ftp_proxy",
            "FTP_PROXY",
            "all_proxy",
            "ALL_PROXY",
            "no_proxy",
            "NO_PROXY",
        ]
        .into_iter()
        .collect();
        keep.extend(self.extra_env.iter().map(String::as_str));
        for name in ["TMP", "TMPDIR"] {
            if inherited
                .get(name)
                .is_some_and(|path| self.usable_temporary_path(path))
            {
                keep.insert(name);
            }
        }
        let mut environment: BTreeMap<_, _> = inherited
            .iter()
            .filter(|(name, _)| keep.contains(name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        environment.insert("LANG".into(), "en_US.UTF-8".into());
        environment.insert(
            "PATH".into(),
            self.binpath.map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{}/bin:{}/sbin:/bin:/sbin:/usr/bin:/usr/sbin",
                    self.prefix, self.prefix
                )
            }),
        );
        environment
    }
    fn usable_temporary_path(&self, path: &str) -> bool {
        let Ok(cpath) = CString::new(path) else {
            return false;
        };
        // SAFETY: cpath is NUL-terminated and valid for the duration of access.
        if unsafe { libc::access(cpath.as_ptr(), libc::W_OK) } != 0 {
            return false;
        }
        // Upstream checks writability and ownership, not directory type. Follow
        // symlinks here just as Tcl file writable and file attributes do.
        self.invoking_uid != 0
            || self.macports_uid == 0
            || std::fs::metadata(path).is_ok_and(|metadata| metadata.uid() == self.macports_uid)
    }
}
