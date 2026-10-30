use port_rs::install::proxy::{self, Settings};
use std::collections::BTreeMap;

fn settings(prefix: &str) -> Settings {
    Settings {
        http: Some(format!("{prefix}-http")),
        https: Some(format!("{prefix}-https")),
        ftp: Some(format!("{prefix}-ftp")),
        rsync: Some(format!("{prefix}-rsync")),
        skip: Some(format!("{prefix}-skip")),
    }
}
fn inherited() -> BTreeMap<String, String> {
    [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "ftp_proxy",
        "FTP_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "no_proxy",
        "NO_PROXY",
        "RSYNC_PROXY",
    ]
    .into_iter()
    .map(|key| (key.into(), key.into()))
    .collect()
}
#[test]
fn environment_wins_by_default_with_lowercase_precedence() {
    let mut env = inherited();
    let before = env.clone();
    let result = proxy::apply(&mut env, &settings("config"), &settings("system"), false);
    assert_eq!(env, before);
    for scheme in ["http", "https", "ftp", "all"] {
        assert_eq!(result.proxies[scheme], format!("{scheme}_proxy"));
    }
    assert_eq!(result.no_proxy.as_deref(), Some("no_proxy"));
    let mut env = BTreeMap::from([("HTTP_PROXY".into(), "ignored".into())]);
    assert!(
        proxy::apply(&mut env, &Settings::default(), &Settings::default(), false)
            .proxies
            .is_empty()
    );
}
#[test]
fn configuration_overrides_system_and_optional_environment_values() {
    for override_env in [false, true] {
        let mut env = if override_env {
            inherited()
        } else {
            BTreeMap::new()
        };
        let result = proxy::apply(
            &mut env,
            &settings("config"),
            &settings("system"),
            override_env,
        );
        for scheme in ["http", "https", "ftp"] {
            assert_eq!(result.proxies[scheme], format!("config-{scheme}"));
        }
        assert_eq!(result.no_proxy.as_deref(), Some("config-skip"));
        assert_eq!(env["RSYNC_PROXY"], "config-rsync");
        assert!(!env.contains_key("https_proxy"));
        if override_env {
            assert_eq!(result.proxies["all"], "all_proxy");
        }
    }
    let mut env = BTreeMap::new();
    let result = proxy::apply(&mut env, &Settings::default(), &settings("system"), false);
    assert_eq!(result.proxies["http"], "system-http");
    assert_eq!(result.proxies["https"], "system-https");
    assert_eq!(result.proxies["ftp"], "system-ftp");
    assert_eq!(result.no_proxy.as_deref(), Some("system-skip"));
    assert!(!env.contains_key("RSYNC_PROXY"));
}
#[test]
fn override_without_replacement_preserves_upstream_case_asymmetry_and_empty_values() {
    let mut env = inherited();
    let result = proxy::apply(&mut env, &Settings::default(), &Settings::default(), true);
    assert_eq!(result.proxies["http"], "http_proxy");
    assert_eq!(result.proxies["https"], "HTTPS_PROXY");
    assert_eq!(result.proxies["ftp"], "FTP_PROXY");
    assert_eq!(result.no_proxy.as_deref(), Some("NO_PROXY"));
    assert_eq!(env["RSYNC_PROXY"], "RSYNC_PROXY");
    for key in ["https_proxy", "ftp_proxy", "no_proxy"] {
        assert!(!env.contains_key(key));
    }
    let mut env = BTreeMap::from([("https_proxy".into(), "lower-only".into())]);
    assert!(
        !proxy::apply(&mut env, &Settings::default(), &Settings::default(), true)
            .proxies
            .contains_key("https")
    );
    let mut env = BTreeMap::from([("http_proxy".into(), String::new())]);
    assert_eq!(
        proxy::apply(&mut env, &settings("config"), &settings("system"), false).proxies["http"],
        ""
    );
    let config = Settings {
        https: Some(String::new()),
        skip: Some(String::new()),
        ..Settings::default()
    };
    let result = proxy::apply(&mut env, &config, &settings("system"), true);
    assert_eq!(result.proxies["https"], "");
    assert_eq!(result.no_proxy.as_deref(), Some(""));
}
