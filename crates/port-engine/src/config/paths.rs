//! Startup configuration locations and path-value normalization.
use super::Config;
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Paths {
    pub configuration_directory: PathBuf,
    pub user_home: Option<PathBuf>,
    pub working_directory: PathBuf,
    pub portsrc: Option<PathBuf>,
}
impl Paths {
    /// Discover resources only in the active ports tree, with no fallback to a
    /// different configured source. Layout is the initialized installation state.
    pub fn select_archive_sites(
        &self,
        config: &Config,
        tree: &Path,
        session: &mut crate::portfile::Session,
        installation: &crate::install::sites::Layout,
        supported: impl FnMut(&str) -> bool,
    ) -> Result<crate::install::sites::Selection> {
        let platform = session.scalar("os.platform")?;
        let major = session
            .scalar("os.major")?
            .parse::<u32>()
            .context("invalid archive-site platform major version")?;
        let resource = self
            .absolute(tree)
            .join("_resources")
            .join(session.scalar("archive_sites.listpath")?)
            .join(session.scalar("archive_sites.listfile")?);
        let tree_sites =
            crate::install::sites::resource::Resource::read(&resource, &platform, major)?
                .sites()?;
        let configured = self.archive_sites(config, &platform, major)?;
        crate::install::sites::Selection::new(
            tree_sites,
            configured,
            installation,
            |path| self.normalize(path),
            supported,
        )
    }
    /// Combine keys from the live archive-site references with pubkeys.conf.
    pub fn archive_site_keys(
        &self,
        config: &Config,
        selection: &crate::install::sites::Selection,
        references: &[String],
        archive_type: &str,
    ) -> Result<crate::install::pubkeys::Trust> {
        let path = config
            .get("pubkeys_conf")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.absolute(&self.configuration_directory)
                    .join("pubkeys.conf")
            });
        let keys = selection.key_paths(references, archive_type, "sig");
        crate::install::pubkeys::load_signify_paths(&path, &self.working_directory, &keys)
    }
    pub fn archive_sites(
        &self,
        config: &Config,
        platform: &str,
        major: u32,
    ) -> Result<crate::install::sites::Sites> {
        let path = config
            .get("archive_sites_conf")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.absolute(&self.configuration_directory)
                    .join("archive_sites.conf")
            });
        crate::install::sites::Sites::read(&self.absolute(&path), platform, major)
    }

    /// Load configured Signify keys together with already selected site keys.
    pub fn archive_keys(
        &self,
        config: &Config,
        site_keys: Vec<crate::install::signature::TrustedKey>,
    ) -> Result<crate::install::pubkeys::Trust> {
        let path = config
            .get("pubkeys_conf")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.absolute(&self.configuration_directory)
                    .join("pubkeys.conf")
            });
        crate::install::pubkeys::load_signify(&path, &self.working_directory, site_keys)
    }

    /// Resolve the startup home without shell utilities. Caller supplies the
    /// real uid and captured environment; HOME precedes SUDO_USER and uid lookup.
    pub fn discover(
        configuration_directory: PathBuf,
        working_directory: PathBuf,
        environment: &BTreeMap<String, String>,
        real_uid: u32,
    ) -> Result<Self> {
        ensure!(
            working_directory.is_absolute(),
            "startup working directory must be absolute"
        );
        let user_home = if let Some(home) = environment.get("HOME") {
            Some(PathBuf::from(home))
        } else if let Some(user) = environment.get("SUDO_USER") {
            Some(
                crate::accounts::by_name(user, false)?
                    .with_context(|| format!("unknown startup user {user}"))?
                    .home
                    .context("startup user has no home directory")?,
            )
        } else if real_uid != 0 {
            Some(
                crate::accounts::by_id(real_uid, false)?
                    .context("startup uid has no account")?
                    .home
                    .context("startup user has no home directory")?,
            )
        } else {
            None
        };
        Ok(Self {
            configuration_directory,
            user_home,
            working_directory,
            portsrc: environment.get("PORTSRC").map(PathBuf::from),
        })
    }
    pub fn load(&self) -> Result<Config> {
        let mut files = vec![
            self.absolute(&self.configuration_directory)
                .join("macports.conf"),
        ];
        if let Some(home) = &self.user_home {
            let directory = self.absolute(home).join(".macports");
            if directory.is_dir() {
                files.push(directory.join("macports.conf"));
            }
        }
        if let Some(path) = &self.portsrc
            && !path.as_os_str().is_empty()
        {
            files.push(self.expand(path.to_str().context("PORTSRC is not UTF-8")?)?);
        }
        Config::read(&files, |value| self.normalize(value))
    }
    /// Match realpath followed by file normalize fallback: existing final
    /// symlinks resolve, while missing leaves still yield an absolute path.
    pub fn normalize(&self, value: &str) -> Result<String> {
        if value.is_empty() {
            return Ok(String::new());
        }
        let expanded = self.expand(value)?;
        let path = match expanded.canonicalize() {
            Ok(path) => path,
            Err(_) => crate::registry::path::normalize(&expanded)?,
        };
        path.into_os_string()
            .into_string()
            .map_err(|_| anyhow::anyhow!("configuration path is not UTF-8"))
    }
    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.working_directory.join(path)
        }
    }
    fn expand(&self, value: &str) -> Result<PathBuf> {
        let Some(tail) = value.strip_prefix('~') else {
            return Ok(self.absolute(Path::new(value)));
        };
        let (name, suffix) = tail.split_once('/').unwrap_or((tail, ""));
        let home = if name.is_empty() {
            self.user_home
                .clone()
                .context("no startup home directory for ~ expansion")?
        } else {
            crate::accounts::by_name(name, false)?
                .with_context(|| format!("unknown user {name} in configuration path"))?
                .home
                .context("user has no home directory")?
        };
        Ok(self.absolute(&home).join(suffix.trim_start_matches('/')))
    }
}
