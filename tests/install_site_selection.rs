use port_rs::install::sites::{Layout, Selection, Sites, resource::Resource};

const SOURCE: &str = include_str!("fixtures/archive-sites/archive_sites.tcl");
fn layout() -> Layout {
    Layout {
        prefix: "/opt/local".into(),
        applications_dir: "/Applications/MacPorts".into(),
        frameworks_dir: "/opt/local/Library/Frameworks".into(),
        cxx_stdlib: "libc++".into(),
        delete_la_files: "yes".into(),
    }
}
fn select(config: &str) -> Selection {
    Selection::new(
        Resource::evaluate(SOURCE, "darwin", 24)
            .unwrap()
            .sites()
            .unwrap(),
        Sites::parse(config, "darwin", 24).unwrap(),
        &layout(),
        |path| Ok(path.to_owned()),
        |kind| matches!(kind, "tbz2" | "txz"),
    )
    .unwrap()
}

#[test]
fn selection_preserves_tree_trust_and_configured_metadata_precedence() {
    let tree = select("");
    assert_eq!(tree.defaults, ["macports_archives::tbz2"]);
    let official = tree.get("macports_archives").unwrap();
    assert_eq!(official.signature_type, "sig");
    assert!(
        official
            .public_key
            .as_ref()
            .unwrap()
            .ends_with("macports-archives-2025.pub")
    );
    assert_eq!(official.urls[0], "https://packages.macports.org/:nosubdir");
    let replaced = select(
        "name macports_archives\nurls https://custom/\ntype txz\nname extra\nurls https://extra/\n",
    );
    assert_eq!(
        replaced.defaults,
        [
            "macports_archives::tbz2",
            "macports_archives::txz",
            "extra::tbz2"
        ]
    );
    let site = replaced.get("macports_archives").unwrap();
    assert_eq!(site.urls, ["https://custom/:nosubdir"]);
    assert_eq!(site.archive_type, "txz");
    assert_eq!(site.signature_type, "rmd160");
    assert!(site.public_key.is_none());
}

#[test]
fn unusable_configured_sites_do_not_hide_compatible_tree_sites() {
    for config in [
        "name macports_archives\nurls\n",
        "name macports_archives\nurls https://custom/\nprefix /different\n",
        "name macports_archives\nurls https://custom/\ntype unsupported\n",
        "name macports_archives\nurls https://custom/\ncxx_stdlib libstdc++\n",
    ] {
        let selected = select(config);
        assert_eq!(selected.defaults, ["macports_archives::tbz2"]);
        assert_eq!(
            selected.get("macports_archives").unwrap().signature_type,
            "sig"
        );
    }
    let tree = Resource::evaluate(SOURCE, "darwin", 24)
        .unwrap()
        .sites()
        .unwrap();
    let mut relocated = layout();
    relocated.prefix = "/elsewhere".into();
    let selected = Selection::new(
        tree,
        Sites::default(),
        &relocated,
        |p| Ok(p.into()),
        |_| true,
    )
    .unwrap();
    assert!(selected.defaults.is_empty());
    assert!(selected.get("macports_archives").is_none());
}

#[test]
fn incomplete_resource_sites_warn_and_do_not_receive_configuration_defaults() {
    let resource = Resource::evaluate("namespace eval ::portfetch::mirror_sites {set archive_prefix(broken) /opt/local; set sites(orphan) https://orphan/}", "darwin", 24).unwrap();
    let sites = resource.sites().unwrap();
    assert!(sites.entries.is_empty());
    assert_eq!(sites.warnings.len(), 5);
    assert!(
        sites
            .warnings
            .iter()
            .all(|warning| warning.contains("'broken'"))
    );
    let selected = Selection::new(
        sites,
        Sites::default(),
        &layout(),
        |p| Ok(p.into()),
        |_| true,
    )
    .unwrap();
    assert_eq!(selected.warnings.len(), 5);
    let resource = Resource::evaluate(&format!("{SOURCE}\nunset ::portfetch::mirror_sites::archive_sigtype\nunset ::portfetch::mirror_sites::archive_pubkey"), "darwin", 24).unwrap();
    let sites = resource.sites().unwrap();
    assert_eq!(sites.entries[0].signature_type, "");
    assert!(sites.entries[0].public_key.is_none());
    let missing_urls = Resource::evaluate(
        &format!("{SOURCE}\nunset ::portfetch::mirror_sites::sites"),
        "darwin",
        24,
    )
    .unwrap();
    assert!(missing_urls.sites().is_err());
}
