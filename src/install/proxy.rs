//! macports::mportinit proxy precedence, without process-global mutation.
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Settings {
    pub http: Option<String>,
    pub https: Option<String>,
    pub ftp: Option<String>,
    pub rsync: Option<String>,
    pub skip: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    /// HTTP, HTTPS, FTP, and ALL entries corresponding to macports::proxies.
    pub proxies: BTreeMap<String, String>,
    pub no_proxy: Option<String>,
}

/// Apply configured and discovered system settings to an already filtered
/// environment. `override_env` is the parsed proxy_override_env option. Empty
/// values are present values, not missing settings. System discovery is supplied
/// by the caller; upstream never uses a system rsync setting here.
pub fn apply(
    environment: &mut BTreeMap<String, String>,
    configured: &Settings,
    system: &Settings,
    override_env: bool,
) -> Resolved {
    if (!environment.contains_key("http_proxy") || override_env)
        && let Some(value) = configured.http.as_ref().or(system.http.as_ref())
    {
        environment.insert("http_proxy".into(), value.clone());
    }
    for (lower, upper, configured, system) in [
        (
            "https_proxy",
            "HTTPS_PROXY",
            &configured.https,
            &system.https,
        ),
        ("ftp_proxy", "FTP_PROXY", &configured.ftp, &system.ftp),
        ("no_proxy", "NO_PROXY", &configured.skip, &system.skip),
    ] {
        if (!environment.contains_key(lower) && !environment.contains_key(upper)) || override_env {
            // Upstream removes the lowercase value even if no replacement is
            // available, while retaining an existing uppercase value.
            environment.remove(lower);
            if let Some(value) = configured.as_ref().or(system.as_ref()) {
                environment.insert(upper.into(), value.clone());
            }
        }
    }
    if (!environment.contains_key("RSYNC_PROXY") || override_env)
        && let Some(value) = &configured.rsync
    {
        environment.insert("RSYNC_PROXY".into(), value.clone());
    }
    let mut proxies = BTreeMap::new();
    if let Some(value) = environment.get("http_proxy") {
        proxies.insert("http".into(), value.clone());
    }
    for scheme in ["https", "ftp", "all"] {
        if let Some(value) = environment
            .get(&format!("{scheme}_proxy"))
            .or_else(|| environment.get(&format!("{}_PROXY", scheme.to_ascii_uppercase())))
        {
            proxies.insert(scheme.into(), value.clone());
        }
    }
    Resolved {
        proxies,
        no_proxy: environment
            .get("no_proxy")
            .or_else(|| environment.get("NO_PROXY"))
            .cloned(),
    }
}

impl Resolved {
    /// Configure HTTP(S) transport exclusively from this resolved snapshot.
    /// A present empty scheme value disables its ALL fallback, as in curl.
    /// reqwest handles proxy authentication and NO_PROXY host/IP matching.
    pub fn configure_client(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> anyhow::Result<reqwest::ClientBuilder> {
        use anyhow::Context;
        let mut builder = builder.no_proxy();
        for scheme in ["http", "https"] {
            if let Some(value) = self.proxies.get(scheme).or_else(|| self.proxies.get("all")) {
                if value.is_empty() {
                    continue;
                }
                let proxy = if scheme == "http" {
                    reqwest::Proxy::http(value)
                } else {
                    reqwest::Proxy::https(value)
                }
                .with_context(|| format!("invalid {scheme} proxy setting"))?
                .no_proxy(
                    self.no_proxy
                        .as_deref()
                        .and_then(reqwest::NoProxy::from_string),
                );
                builder = builder.proxy(proxy);
            }
        }
        Ok(builder)
    }
}
