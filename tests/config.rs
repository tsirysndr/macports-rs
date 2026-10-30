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
