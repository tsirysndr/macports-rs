use port_rs::{
    index::Port,
    install::{
        Action, Item,
        sites::{
            requests::{Subdirectories, resolve_mirror},
            resource::Resource,
        },
        target::{Architecture, Target},
    },
    source::Platform,
};
use std::collections::BTreeMap;

#[test]
fn mirror_options_ports_ipv6_and_name_substitution_follow_portlib_rules() {
    for (input, subdir, expected) in [
        ("https://host/", "", "https://host/curl"),
        ("https://host/:nosubdir", "", "https://host/"),
        ("https://host/:nosubdir", "custom/", "https://host/custom/"),
        ("https://host/:mirror", "ignored", "https://host/dist/curl"),
        ("https://host/$name/:nosubdir", "", "https://host/curl/"),
        (
            "https://host/${name}/$name",
            "custom",
            "https://host/custom/$name",
        ),
        (
            "https://host/$name_suffix/",
            "",
            "https://host/$name_suffix/curl",
        ),
        ("https://host:8080", "", "https://host:8080curl"),
        ("https://[::1]:8080/:nosubdir", "", "https://[::1]:8080/"),
    ] {
        assert_eq!(
            resolve_mirror(input, subdir, "curl", "dist/curl"),
            expected,
            "{input}"
        );
    }
}

#[test]
fn selected_official_mirrors_make_the_same_coordinates_as_official_target() {
    let resource = Resource::evaluate(
        include_str!("fixtures/archive-sites/archive_sites.tcl"),
        "darwin",
        24,
    )
    .unwrap();
    let mut sites = resource.sites().unwrap();
    let site = &mut sites.entries[0];
    let port = Port {
        name: "curl".into(),
        fields: BTreeMap::from([
            ("version".into(), "8.23.0".into()),
            ("revision".into(), "0".into()),
        ]),
    };
    let item = Item {
        port: &port,
        variants: "+ssl".into(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    };
    let target = Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        "darwin",
        Architecture::Native,
    )
    .unwrap();
    let subdirs = Subdirectories {
        port_name: "curl",
        mirror: "",
        archive: "curl",
        dist: "curl",
    };
    let request = site.signify_request(&target, &item, &subdirs, 100).unwrap();
    assert_eq!(request.name, "curl");
    assert_eq!(request.max_bytes, 100);
    assert_eq!(request.urls[0], target.request(&item, 100).unwrap().urls[0]);
    assert_eq!(request.urls.len(), site.urls.len());
    assert!(request.urls.iter().all(|url| !url.contains(":nosubdir")));
    site.urls = vec!["https://host/prefix/:nosubdir".into()];
    let direct = Subdirectories {
        port_name: "curl",
        mirror: "",
        archive: "",
        dist: "curl",
    };
    assert_eq!(
        site.signify_request(&target, &item, &direct, 100)
            .unwrap()
            .urls[0],
        format!("https://host/prefix/{}", target.filename(&item).unwrap())
    );
    for url in [
        "file:///tmp/:nosubdir",
        "https://user:password@host/:nosubdir",
        "https://host/?query:nosubdir",
        "https://host/#fragment:nosubdir",
    ] {
        site.urls = vec![url.into()];
        assert!(site.signify_request(&target, &item, &subdirs, 100).is_err());
    }
    site.urls = vec!["https://host/:nosubdir".into()];
    site.signature_type = "rmd160".into();
    assert!(site.signify_request(&target, &item, &subdirs, 100).is_err());
    site.signature_type = "sig".into();
    site.archive_type = "txz".into();
    assert!(site.signify_request(&target, &item, &subdirs, 100).is_err());
}

#[test]
fn live_session_selects_mirrors_and_keeps_subport_archive_directory_distinct() {
    use port_rs::{
        install::sites::{Layout, Selection, Sites},
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    let resource = Resource::evaluate(
        include_str!("fixtures/archive-sites/archive_sites.tcl"),
        "darwin",
        24,
    )
    .unwrap();
    let selection = Selection::new(
        resource.sites().unwrap(),
        Sites::default(),
        &Layout {
            prefix: "/opt/local".into(),
            applications_dir: "/Applications/MacPorts".into(),
            frameworks_dir: "/opt/local/Library/Frameworks".into(),
            cxx_stdlib: "libc++".into(),
            delete_la_files: "yes".into(),
        },
        |path| Ok(path.into()),
        |_| true,
    )
    .unwrap();
    let mut session = Session::open(
        "name parent; version 1; subport child {}",
        &BTreeMap::new(),
        &PortSpec::named("child"),
        PortGroups::default(),
    )
    .unwrap();
    let port = session.metadata().port.clone();
    let item = Item {
        port: &port,
        variants: "".into(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    };
    let target = Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        "darwin",
        Architecture::Native,
    )
    .unwrap();
    let prepared = selection
        .request_from_session(&mut session, &target, &item, 100)
        .unwrap();
    assert_eq!(prepared.references, selection.defaults);
    assert_eq!(
        prepared.request.urls[0],
        format!(
            "https://packages.macports.org/child/{}",
            target.filename(&item).unwrap()
        )
    );
    session
        .invoke("archive_sites", &["macports_archives:custom/:tbz2"])
        .unwrap();
    session
        .invoke("archive_sites.mirror_subdir", &["nested/"])
        .unwrap();
    session.invoke("archive.subdir", &["override"]).unwrap();
    let prepared = selection
        .request_from_session(&mut session, &target, &item, 100)
        .unwrap();
    assert_eq!(prepared.references, ["macports_archives:custom/:tbz2"]);
    assert_eq!(
        prepared.request.urls[0],
        format!(
            "https://packages.macports.org/custom/nested/override/{}",
            target.filename(&item).unwrap()
        )
    );
    session.invoke("archive_sites", &[]).unwrap();
    assert!(
        selection
            .request_from_session(&mut session, &target, &item, 100)
            .is_err()
    );
    session.invoke("archive_sites", &["missing::tbz2"]).unwrap();
    assert!(
        selection
            .request_from_session(&mut session, &target, &item, 100)
            .is_err()
    );
}
