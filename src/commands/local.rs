//! Shared local-tree selection and explicit native evaluation variables.
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn root(index: Option<&Path>, indexed: bool, action: &str) -> Result<Option<PathBuf>> {
    if indexed {
        return Ok(None);
    }
    if let Some(path) = index {
        return Ok(Some(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf(),
        ));
    }
    ensure!(
        Path::new("PortIndex").try_exists()?,
        "no local ports tree selected; use --index-file PATH or use {action} --index"
    );
    Ok(Some(PathBuf::from(".")))
}

fn host_variables(root: Option<&Path>) -> Result<BTreeMap<String, String>> {
    if root.is_none() {
        return Ok(BTreeMap::new());
    }
    crate::source::detect_build_host()?.variables()
}

/// Configuration and Portfile-session setup shared by local-tree actions.
pub(super) struct Host {
    pub context: crate::config::Context,
    pub archives: crate::portfile::ArchiveSites,
    pub registry_path: PathBuf,
    pub progress: crate::progress::Progress,
    pub paths: crate::config::Paths,
    pub config: crate::config::Config,
}

impl Host {
    pub fn new(root: &Path, registry_file: Option<&Path>, quiet: bool) -> Result<Self> {
        let variables = host_variables(Some(root))?;
        let environment = std::env::vars_os()
            .map(|(name, value)| {
                Ok((
                    name.into_string()
                        .map_err(|_| anyhow::anyhow!("startup environment name is not UTF-8"))?,
                    value
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("startup environment value is not UTF-8"))?,
                ))
            })
            .collect::<Result<_>>()?;
        // SAFETY: getuid has no arguments and does not mutate process state.
        let uid = unsafe { libc::getuid() };
        let paths = crate::config::Paths::discover(
            "/opt/local/etc/macports".into(),
            std::env::current_dir()?,
            &environment,
            uid,
        )?;
        let config = paths.load()?;
        let context = crate::config::Context::new(
            &config,
            variables,
            &environment,
            uid,
            &crate::install::proxy::Settings::default(),
        )?;
        let progress = crate::progress::Progress::new(quiet);
        for warning in &context.warnings {
            progress.warning(warning);
        }
        let registry_path = context.registry_path(registry_file)?;
        let archives = paths.archive_resolver(&config, &context, root, progress.clone())?;
        Ok(Self {
            context,
            archives,
            registry_path,
            progress,
            paths,
            config,
        })
    }

    pub fn open(
        &self,
        root: &Path,
        port: &crate::index::Port,
        spec: &crate::spec::PortSpec,
    ) -> Result<crate::portfile::Session> {
        let mut session = crate::portfile::open_from_tree_with_registry(
            root,
            port,
            &self.context.variables,
            spec,
            &self.context.environment,
            self.archives.clone(),
            Some(&self.registry_path),
        )?;
        session.set_mirror_timings(self.context.mirror_timings.clone());
        Ok(session)
    }

    /// Metadata for one port, evaluated as `install` would open it.
    pub fn evaluate(
        &self,
        root: &Path,
        port: &crate::index::Port,
        spec: &crate::spec::PortSpec,
    ) -> Result<crate::portfile::Evaluation> {
        Ok(self.open(root, port, spec)?.into_metadata())
    }
}

/// macports.conf as layered upstream (system file, ~/.macports, PORTSRC).
pub(super) fn configuration() -> Option<crate::config::Config> {
    let environment: BTreeMap<String, String> = std::env::vars().collect();
    // SAFETY: getuid has no arguments and does not mutate process state.
    let uid = unsafe { libc::getuid() };
    let paths = crate::config::Paths::discover(
        "/opt/local/etc/macports".into(),
        std::env::current_dir().ok()?,
        &environment,
        uid,
    )
    .ok()?;
    paths.load().ok()
}

pub(super) fn configured_prefix() -> PathBuf {
    configuration()
        .and_then(|config| config.get("prefix").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/opt/local"))
}

/// The registry selected by macports.conf (`portdbpath`, or `${prefix}/var/macports`).
pub(super) fn configured_registry() -> Option<PathBuf> {
    let config = configuration()?;
    let database = match config.get("portdbpath") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(config.get("prefix")?).join("var/macports"),
    };
    Some(database.join("registry/registry.db"))
}
