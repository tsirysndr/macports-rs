//! Native macports.conf data parsing. Configuration values are never executed.
use anyhow::{Context as _, Result};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

const OPTIONS: &str = "binpath auto_path clonebin_path extra_env portdbformat portarchivetype portimage_mode hfscompression portautoclean porttrace portverbose keeplogs destroot_umask release_urls release_version_urls rsync_server rsync_options rsync_dir startupitem_autostart startupitem_type startupitem_install place_worksymlink xcodeversion xcodebuildcmd xcodecltversion xcode_license_unaccepted configureccache ccache_size configuredistcc configurepipe buildnicevalue buildmakejobs universal_archs build_arch macosx_sdk_version macosx_deployment_target macportsuser proxy_override_env proxy_http proxy_https proxy_ftp proxy_rsync proxy_skip master_site_local patch_site_local archive_site_local fetch_credentials fetch_threads buildfromsource revupgrade_autorun revupgrade_mode revupgrade_check_id_loadcmds host_blacklist preferred_hosts sandbox_enable sandbox_network delete_la_files cxx_stdlib default_compilers pkg_post_unarchive_deletions ui_interactive";
const PATHS: &str = "applications_dir archive_sites_conf ccache_dir developer_dir frameworks_dir packagemaker_path portdbpath prefix pubkeys_conf sources_conf variants_conf";

#[derive(Clone, Default, Debug)]
pub struct Config {
    values: BTreeMap<String, String>,
}
impl Config {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    /// Read an ordered file list (system, user, PORTSRC); later settings win.
    /// Missing files are ignored. The caller supplies path normalization with
    /// its startup HOME/current-directory context, including named-user tildes.
    pub fn read(
        paths: &[impl AsRef<Path>],
        mut normalize: impl FnMut(&str) -> Result<String>,
    ) -> Result<Self> {
        let mut config = Self::default();
        for path in paths {
            let path = path.as_ref();
            let source = match std::fs::read_to_string(path) {
                Ok(source) => source,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("cannot read configuration {}", path.display()));
                }
            };
            config
                .apply(&source, &mut normalize)
                .with_context(|| format!("configuration {}", path.display()))?;
        }
        Ok(config)
    }

    /// Apply one file. Invalid/unknown option lines are ignored as upstream;
    /// malformed lists in a recognized continuation remain errors.
    pub fn apply(
        &mut self,
        source: &str,
        mut normalize: impl FnMut(&str) -> Result<String>,
    ) -> Result<()> {
        static LINE: OnceLock<regex::Regex> = OnceLock::new();
        let pattern = LINE.get_or_init(|| regex::Regex::new(r"^(\w+)([ \t]+(.*))?$").unwrap());
        let mut continuation = false;
        let mut option = String::new();
        for (index, line) in source.lines().enumerate() {
            let next_continuation = line.ends_with('\\');
            let line = line.strip_suffix('\\').unwrap_or(line);
            let value = if continuation {
                line
            } else {
                let Some(captures) = pattern.captures(line) else {
                    continue;
                };
                option = captures[1].into();
                captures.get(3).map_or("", |value| value.as_str())
            };
            let is_path = PATHS.split_whitespace().any(|name| name == option);
            if is_path || OPTIONS.split_whitespace().any(|name| name == option) {
                let value = value.trim();
                let parsed = (|| -> Result<String> {
                    if is_path {
                        return normalize(value);
                    }
                    if !continuation {
                        return Ok(value.into());
                    }
                    let mut elements =
                        crate::tcl_list::parse(self.get(&option).unwrap_or_default())?;
                    elements.extend(crate::tcl_list::parse(value)?);
                    Ok(molt::Value::from(
                        elements
                            .into_iter()
                            .map(molt::Value::from)
                            .collect::<Vec<_>>(),
                    )
                    .to_string())
                })()
                .with_context(|| format!("line {}: {option}", index + 1))?;
                self.values.insert(option.clone(), parsed);
            }
            continuation = next_continuation;
        }
        Ok(())
    }

    pub fn extra_env(&self) -> Result<Vec<String>> {
        crate::tcl_list::parse(self.get("extra_env").unwrap_or_default())
    }
    pub fn proxy_settings(&self) -> crate::install::proxy::Settings {
        crate::install::proxy::Settings {
            http: self.get("proxy_http").map(str::to_owned),
            https: self.get("proxy_https").map(str::to_owned),
            ftp: self.get("proxy_ftp").map(str::to_owned),
            rsync: self.get("proxy_rsync").map(str::to_owned),
            skip: self.get("proxy_skip").map(str::to_owned),
        }
    }
}

mod archives;
mod paths;
pub use paths::Paths;

mod context;
pub use context::Context;

mod variants;
pub use variants::Variants;
