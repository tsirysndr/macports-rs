//! Assemble configured Portfile inputs without creating directories or a registry.
use super::Config;
use crate::install::{
    environment::Startup,
    proxy::{self, Resolved, Settings},
};
use anyhow::{Context as _, Result};
use std::collections::BTreeMap;

pub struct Context {
    pub variables: BTreeMap<String, String>,
    pub environment: BTreeMap<String, String>,
    pub proxies: Resolved,
    pub mirror_timings: crate::install::mirror_times::MirrorTimings,
    pub warnings: Vec<String>,
    pub variants: super::Variants,
}
impl Context {
    /// Open a fresh read-only snapshot from the configured database directory.
    /// Explicit overrides remain strict: a missing override is an error. An
    /// absent configured registry represents an empty installation, and is never
    /// created by evaluation or previews.
    pub fn open_registry(
        &self,
        explicit: Option<&std::path::Path>,
    ) -> Result<Option<crate::registry::Registry>> {
        let path = self.registry_path(explicit)?;
        if explicit.is_none() && !path.try_exists()? {
            return Ok(None);
        }
        crate::registry::Registry::open(Some(&path))
    }

    /// Selected path even when not created yet, allowing later fresh lookups.
    pub fn registry_path(&self, explicit: Option<&std::path::Path>) -> Result<std::path::PathBuf> {
        if let Some(path) = explicit {
            return Ok(path.to_owned());
        }
        Ok(std::path::Path::new(
            self.variables
                .get("portdbpath")
                .context("startup portdbpath is missing")?,
        )
        .join("registry/registry.db"))
    }

    /// Host variables supply standalone defaults; explicit configuration wins.
    /// This assembles evaluation inputs, not PortSystem target initialization.
    pub fn new(
        config: &Config,
        mut variables: BTreeMap<String, String>,
        inherited: &BTreeMap<String, String>,
        real_uid: u32,
        system_proxies: &Settings,
    ) -> Result<Self> {
        variables.extend(config.values().clone());
        let archive_type = match variables.get("portarchivetype") {
            Some(value) => crate::tcl_list::parse(value)
                .context("invalid portarchivetype list")?
                .into_iter()
                .next()
                .unwrap_or_default(),
            None => "tbz2".into(),
        };
        variables.insert("portarchivetype".into(), archive_type);
        let prefix = variables
            .get("prefix")
            .context("startup prefix is missing")?
            .clone();
        variables
            .entry("portdbpath".into())
            .or_insert_with(|| format!("{prefix}/var/macports"));
        variables
            .entry("frameworks_dir".into())
            .or_insert_with(|| format!("{prefix}/Library/Frameworks"));
        variables
            .entry("applications_dir".into())
            .or_insert_with(|| "/Applications/MacPorts".into());
        let portdbpath = variables["portdbpath"].clone();
        variables
            .entry("ccache_dir".into())
            .or_insert_with(|| format!("{portdbpath}/build/.ccache"));
        let darwin_major = variables
            .get("os.major")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        for (name, value) in [
            ("configureccache", "no"),
            ("configuredistcc", "no"),
            ("configurepipe", "yes"),
            ("buildnicevalue", "0"),
            ("buildmakejobs", "0"),
            ("ccache_size", "2G"),
            ("revupgrade_autorun", "yes"),
            ("revupgrade_mode", "rebuild"),
            ("destroot_umask", "022"),
            ("clonebin_path", ""),
            ("startupitem_type", "default"),
            ("startupitem_install", "yes"),
            ("startupitem_autostart", "yes"),
            ("place_worksymlink", "yes"),
            ("sandbox_enable", "yes"),
            ("hfscompression", "yes"),
            ("portautoclean", "yes"),
            ("keeplogs", "no"),
            ("portverbose", "no"),
            ("rsync_server", "rsync.macports.org"),
            ("rsync_dir", "macports/release/tarballs/base.tar"),
            ("rsync_options", "-rtzvl --delete-after"),
            (
                "delete_la_files",
                if darwin_major >= 13 { "yes" } else { "no" },
            ),
            (
                "cxx_stdlib",
                if darwin_major >= 10 {
                    "libc++"
                } else {
                    "libstdc++"
                },
            ),
        ] {
            variables.entry(name.into()).or_insert_with(|| value.into());
        }
        variables
            .entry("portsharepath".into())
            .or_insert_with(|| format!("{prefix}/share/macports"));
        for name in ["prefix", "frameworks_dir", "applications_dir"] {
            variables.insert(format!("{name}_frozen"), variables[name].clone());
        }
        let user = variables
            .entry("macportsuser".into())
            .or_insert_with(|| "macports".into())
            .clone();
        let mut variants =
            super::Variants::read(config.get("variants_conf").map(std::path::Path::new))?;
        let mut warnings = std::mem::take(&mut variants.warnings);
        let build_uid = if real_uid != 0 || user == "root" {
            0
        } else {
            let account = crate::accounts::by_name(&user, false)?;
            let group = crate::accounts::by_name(&user, true)?;
            match (account, group) {
                (Some(account), Some(_)) => account.id,
                _ => {
                    warnings.push(format!(
                        "configured user/group {user} does not exist, will build as root"
                    ));
                    variables.insert("macportsuser".into(), "root".into());
                    0
                }
            }
        };
        let extra = config.extra_env()?;
        let mut environment = Startup {
            prefix: &prefix,
            binpath: config.get("binpath"),
            extra_env: &extra,
            invoking_uid: real_uid,
            macports_uid: build_uid,
        }
        .prepare(inherited);
        let override_env = config.get("proxy_override_env").is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "t" | "tr" | "tru" | "true" | "y" | "ye" | "yes" | "on"
            )
        });
        let proxies = proxy::apply(
            &mut environment,
            &config.proxy_settings(),
            system_proxies,
            override_env,
        );
        environment.insert("CCACHE_DIR".into(), variables["ccache_dir"].clone());
        let mirror_timings =
            crate::install::mirror_times::MirrorTimings::from_variables(&variables)?;
        Ok(Self {
            mirror_timings,
            variables,
            environment,
            proxies,
            warnings,
            variants,
        })
    }
}
