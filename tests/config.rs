use port_rs::{config::Config, tcl_list};

#[test]
fn literal_values_comments_unknown_options_and_repeated_settings_follow_upstream() {
    let mut config = Config::default();
    config.apply("# comment\n prefix ignored\nunknown value\\\n binpath ignored continuation\nbinpath /first\nbinpath /last # literal\nproxy_http $env(PROXY); [error nope]\nproxy_https\n", |_| panic!("no recognized path")).unwrap();
    assert_eq!(config.get("prefix"), None);
    assert_eq!(config.get("unknown"), None);
    assert_eq!(config.get("binpath"), Some("/last # literal"));
    assert_eq!(config.get("proxy_http"), Some("$env(PROXY); [error nope]"));
    assert_eq!(config.proxy_settings().https.as_deref(), Some(""));
}
#[test]
fn continuations_append_list_elements_but_replace_path_options() {
    let mut config = Config::default();
    config
        .apply(
            "extra_env A {B C}\\\n  D {E F}\\\n G\nprefix /one\\\n /two\n",
            |value| Ok(format!("normalized:{value}")),
        )
        .unwrap();
    assert_eq!(config.extra_env().unwrap(), ["A", "B C", "D", "E F", "G"]);
    assert_eq!(config.get("prefix"), Some("normalized:/two"));
    config
        .apply("extra_env only\\\n# literal continuation\n", |value| {
            Ok(value.into())
        })
        .unwrap();
    assert_eq!(
        config.extra_env().unwrap(),
        ["only", "#", "literal", "continuation"]
    );
    let error = config
        .apply("extra_env A\\\n{broken\n", |value| Ok(value.into()))
        .unwrap_err();
    assert!(format!("{error:#}").contains("line 2: extra_env"));
    let mut eof = Config::default();
    eof.apply("extra_env A\\", |value| Ok(value.into()))
        .unwrap();
    assert_eq!(
        tcl_list::parse(eof.get("extra_env").unwrap()).unwrap(),
        ["A"]
    );
}
#[test]
fn ordered_files_override_values_and_missing_files_are_ignored() {
    let root = tempfile::tempdir().unwrap();
    let system = root.path().join("system.conf");
    let user = root.path().join("user.conf");
    let override_file = root.path().join("portsrc");
    std::fs::write(
        &system,
        "binpath /system\nextra_env SYSTEM\nproxy_http system\n",
    )
    .unwrap();
    std::fs::write(&user, "binpath /user\nextra_env USER\\\n MORE\n").unwrap();
    std::fs::write(&override_file, "proxy_http final\n").unwrap();
    let config = Config::read(
        &[system, root.path().join("missing"), user, override_file],
        |value| Ok(value.into()),
    )
    .unwrap();
    assert_eq!(config.get("binpath"), Some("/user"));
    assert_eq!(config.extra_env().unwrap(), ["USER", "MORE"]);
    assert_eq!(config.proxy_settings().http.as_deref(), Some("final"));
}

#[test]
fn discovered_files_and_values_use_explicit_home_and_working_directory() {
    use port_rs::config::Paths;
    use std::collections::BTreeMap;
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    std::fs::create_dir_all(cwd.join("etc")).unwrap();
    std::fs::create_dir_all(cwd.join("home/.macports")).unwrap();
    std::fs::create_dir_all(cwd.join("actual")).unwrap();
    std::os::unix::fs::symlink(cwd.join("actual"), cwd.join("alias")).unwrap();
    std::fs::write(
        cwd.join("etc/macports.conf"),
        "prefix alias\nproxy_http system\n",
    )
    .unwrap();
    std::fs::write(
        cwd.join("home/.macports/macports.conf"),
        "proxy_http user\nframeworks_dir ~/Frameworks\n",
    )
    .unwrap();
    std::fs::write(
        cwd.join("override"),
        "proxy_http last\nportdbpath alias/new/../database\n",
    )
    .unwrap();
    let paths = Paths::discover(
        "etc".into(),
        cwd.clone(),
        &BTreeMap::from([
            ("HOME".into(), "home".into()),
            ("PORTSRC".into(), "override".into()),
            ("SUDO_USER".into(), "nonexistent-user-ignored".into()),
        ]),
        0,
    )
    .unwrap();
    let config = paths.load().unwrap();
    assert_eq!(config.get("prefix"), cwd.join("actual").to_str());
    assert_eq!(
        config.get("frameworks_dir"),
        cwd.join("home/Frameworks").to_str()
    );
    assert_eq!(
        config.get("portdbpath"),
        cwd.join("actual/database").to_str()
    );
    assert_eq!(config.proxy_settings().http.as_deref(), Some("last"));
    assert_eq!(paths.normalize("").unwrap(), "");
    assert_eq!(
        paths.normalize("~//Frameworks").unwrap(),
        cwd.join("home/Frameworks").to_str().unwrap()
    );
    let empty_override = Paths::discover(
        "etc".into(),
        cwd.clone(),
        &BTreeMap::from([("PORTSRC".into(), String::new())]),
        0,
    )
    .unwrap();
    assert_eq!(
        empty_override
            .load()
            .unwrap()
            .proxy_settings()
            .http
            .as_deref(),
        Some("system")
    );
    assert!(paths.normalize("~port-rs-nonexistent-user/path").is_err());
    let no_home = Paths::discover("etc".into(), cwd, &BTreeMap::new(), 0).unwrap();
    assert!(no_home.user_home.is_none());
    assert!(no_home.normalize("~/file").is_err());
    assert_eq!(
        no_home.load().unwrap().proxy_settings().http.as_deref(),
        Some("system")
    );
}

#[test]
fn startup_home_can_be_resolved_through_native_accounts() {
    use port_rs::config::Paths;
    use std::collections::BTreeMap;
    let cwd = std::env::current_dir().unwrap();
    let paths = Paths::discover(
        "/opt/local/etc/macports".into(),
        cwd,
        &BTreeMap::from([("SUDO_USER".into(), "root".into())]),
        0,
    )
    .unwrap();
    assert!(paths.user_home.as_ref().unwrap().is_absolute());
    assert_eq!(
        paths.normalize("~root").unwrap(),
        paths.normalize("~").unwrap()
    );
}

#[test]
fn context_combines_configuration_environment_and_missing_build_user_fallback() {
    use std::collections::BTreeMap;
    let mut config = Config::default();
    config.apply("prefix /configured\nportdbpath /database\nmacportsuser port-rs-nonexistent-account\nextra_env KEEP\nproxy_override_env y\nproxy_http configured-proxy\n", |path| Ok(path.into())).unwrap();
    let context = port_rs::config::Context::new(
        &config,
        BTreeMap::from([("prefix".into(), "/default".into())]),
        &BTreeMap::from([
            ("KEEP".into(), "kept".into()),
            ("DROP".into(), "removed".into()),
            ("http_proxy".into(), "inherited".into()),
        ]),
        0,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(context.variables["prefix_frozen"], "/configured");
    assert_eq!(
        context.variables["frameworks_dir"],
        "/configured/Library/Frameworks"
    );
    assert_eq!(context.variables["macportsuser"], "root");
    assert_eq!(context.warnings.len(), 1);
    assert_eq!(context.environment["CCACHE_DIR"], "/database/build/.ccache");
    assert_eq!(context.environment["KEEP"], "kept");
    assert!(!context.environment.contains_key("DROP"));
    assert_eq!(context.proxies.proxies["http"], "configured-proxy");
    config
        .apply("extra_env {broken", |path| Ok(path.into()))
        .unwrap();
    assert!(
        port_rs::config::Context::new(
            &config,
            BTreeMap::new(),
            &BTreeMap::new(),
            501,
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn archive_type_startup_default_and_first_list_element_match_macports() {
    use std::collections::BTreeMap;
    for (setting, expected) in [(None, "tbz2"), (Some("txz tbz2"), "txz"), (Some(""), "")] {
        let mut config = Config::default();
        if let Some(value) = setting {
            config
                .apply(&format!("portarchivetype {value}"), |path| Ok(path.into()))
                .unwrap();
        }
        let context = port_rs::config::Context::new(
            &config,
            BTreeMap::from([("prefix".into(), "/opt/local".into())]),
            &BTreeMap::new(),
            501,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(context.variables["portarchivetype"], expected);
    }
    let mut config = Config::default();
    config
        .apply("portarchivetype {broken", |path| Ok(path.into()))
        .unwrap();
    assert!(
        port_rs::config::Context::new(
            &config,
            BTreeMap::from([("prefix".into(), "/opt/local".into())]),
            &BTreeMap::new(),
            501,
            &Default::default()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("portarchivetype")
    );
}

#[test]
fn context_shares_configured_mirror_policy_and_measurements() {
    use std::{collections::BTreeMap, time::SystemTime};
    let mut config = port_rs::config::Config::default();
    config
        .apply(
            "preferred_hosts *.example\nhost_blacklist bad.example\n",
            |p| Ok(p.into()),
        )
        .unwrap();
    let context = port_rs::config::Context::new(
        &config,
        BTreeMap::from([("prefix".into(), "/opt/local".into())]),
        &BTreeMap::new(),
        501,
        &port_rs::install::proxy::Settings::default(),
    )
    .unwrap();
    let shared = context.mirror_timings.clone();
    let now = SystemTime::now();
    assert_eq!(shared.lookup("good.example", now), (2, None));
    assert_eq!(shared.lookup("bad.example", now), (-2, None));
    context
        .mirror_timings
        .record("measured", 12.0, now)
        .unwrap();
    assert_eq!(shared.lookup("measured", now), (1, Some(12.0)));
}
