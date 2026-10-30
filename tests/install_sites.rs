use port_rs::install::sites::{Layout, Sites};

#[test]
fn site_defaults_duplicates_and_literal_values_follow_configuration_rules() {
    let sites = Sites::parse("urls ignored\nname custom\nprefix /custom\nurls {https://one/a path/} https://two/\nname custom\ntype txz\nname empty\nsigtype sig\n", "darwin", 24).unwrap();
    assert_eq!(sites.entries.len(), 2);
    let site = &sites.entries[0];
    assert_eq!(site.name, "custom");
    assert_eq!(site.urls, ["https://one/a path/", "https://two/"]);
    assert_eq!(site.archive_type, "txz");
    assert_eq!(site.signature_type, "rmd160");
    assert_eq!(site.layout.frameworks_dir, "/custom/Library/Frameworks");
    assert_eq!(site.layout.cxx_stdlib, "libc++");
    assert_eq!(site.layout.delete_la_files, "yes");
    assert!(sites.entries[1].urls.is_empty());
    assert_eq!(sites.warnings.len(), 3);
    let old = Sites::parse("name old\nurls https://example/", "darwin", 12).unwrap();
    assert_eq!(old.entries[0].layout.cxx_stdlib, "libstdc++");
    assert_eq!(old.entries[0].layout.delete_la_files, "no");
    assert!(
        Sites::parse(
            "name broken\nurls {broken\nurls https://replacement/",
            "darwin",
            24
        )
        .is_err()
    );
}
#[test]
fn compatibility_requires_matching_layout_policy_and_supported_archive_type() {
    let sites = Sites::parse("name test\nurls https://example/", "darwin", 24).unwrap();
    let site = &sites.entries[0];
    let mut installation = Layout {
        prefix: "/opt/local".into(),
        frameworks_dir: "/opt/local/Library/Frameworks".into(),
        applications_dir: "/Applications/MacPorts".into(),
        cxx_stdlib: "libc++".into(),
        delete_la_files: "yes".into(),
    };
    let normalize = |value: &str| Ok(value.to_owned());
    assert!(
        site.compatible(&installation, normalize, |kind| kind == "tbz2")
            .unwrap()
    );
    assert!(
        !site
            .compatible(&installation, normalize, |_| false)
            .unwrap()
    );
    installation.delete_la_files = "no".into();
    assert!(!site.compatible(&installation, normalize, |_| true).unwrap());
    installation.delete_la_files = "yes".into();
    installation.cxx_stdlib = "libstdc++".into();
    assert!(!site.compatible(&installation, normalize, |_| true).unwrap());
    installation.cxx_stdlib = "libc++".into();
    installation.prefix = "/different".into();
    assert!(!site.compatible(&installation, normalize, |_| true).unwrap());
    assert!(
        site.compatible(
            &installation,
            |_| Err(anyhow::anyhow!("cannot normalize")),
            |_| true
        )
        .is_err()
    );
}
#[test]
fn configured_site_file_overrides_the_default_and_missing_files_are_empty() {
    use port_rs::config::{Config, Paths};
    let root = tempfile::tempdir().unwrap();
    let paths = Paths {
        configuration_directory: root.path().to_owned(),
        working_directory: root.path().to_owned(),
        user_home: None,
        portsrc: None,
    };
    assert!(
        paths
            .archive_sites(&Config::default(), "darwin", 24)
            .unwrap()
            .entries
            .is_empty()
    );
    std::fs::write(
        root.path().join("custom"),
        "name selected\nurls https://example/\n",
    )
    .unwrap();
    let mut config = Config::default();
    config
        .apply("archive_sites_conf custom", |path| {
            Ok(root.path().join(path).to_str().unwrap().into())
        })
        .unwrap();
    assert_eq!(
        paths.archive_sites(&config, "darwin", 24).unwrap().entries[0].name,
        "selected"
    );
}
