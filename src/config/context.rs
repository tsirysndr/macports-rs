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
    pub warnings: Vec<String>,
    pub variants: super::Variants,
}
impl Context {
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
        Ok(Self {
            variables,
            environment,
            proxies,
            warnings,
            variants,
        })
    }
}
