use port_rs::{
    index::Port,
    install::{
        Action, Item,
        target::{Architecture, Target},
    },
    source::Platform,
};
use std::collections::BTreeMap;
fn machine(arch: &'static str) -> Platform {
    Platform {
        darwin_major: 24,
        architecture: arch,
    }
}
fn port() -> Port {
    Port {
        name: "curl".into(),
        fields: BTreeMap::from([
            ("version".into(), "8.23.0".into()),
            ("revision".into(), "0".into()),
        ]),
    }
}
fn item(port: &Port) -> Item<'_> {
    Item {
        port,
        variants: "+ssl".into(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    }
}
#[test]
fn native_targets_use_machine_arch_and_darwin_release() {
    let port = port();
    let item = item(&port);
    for (machine_arch, archive_arch) in [("arm", "arm64"), ("i386", "x86_64")] {
        let target = Target::macos(
            &machine(machine_arch),
            "darwin freebsd",
            Architecture::Native,
        )
        .unwrap();
        assert_eq!(
            target.filename(&item).unwrap(),
            format!("curl-8.23.0_0+ssl.darwin_24.{archive_arch}.tbz2")
        );
        assert_eq!(
            target.request(&item, 100).unwrap().urls,
            [format!(
                "https://packages.macports.org/curl/curl-8.23.0_0+ssl.darwin_24.{archive_arch}.tbz2"
            )]
        );
    }
}
#[test]
fn platform_independence_matches_upstream_list_order() {
    for (platforms, expected) in [
        ("any", "any_any"),
        ("{darwin any}", "darwin_any"),
        ("{macosx any >= 13}", "darwin_any"),
        ("{freebsd any}", "darwin_24"),
        ("{darwin any} any", "darwin_any"),
        ("any {darwin any}", "any_any"),
        ("{darwin >= 20}", "darwin_24"),
    ] {
        let target = Target::macos(&machine("arm"), platforms, Architecture::Noarch).unwrap();
        assert_eq!(target.platform(), expected);
        assert_eq!(target.architectures(), ["noarch"]);
    }
}
#[test]
fn universal_architectures_are_sorted_and_explicit_build_arch_is_respected() {
    let target = Target::macos(
        &machine("arm"),
        "darwin",
        Architecture::Universal(&["x86_64", "arm64"]),
    )
    .unwrap();
    assert_eq!(target.architectures(), ["arm64", "x86_64"]);
    assert!(
        target
            .filename(&item(&port()))
            .unwrap()
            .ends_with(".arm64-x86_64.tbz2")
    );
    let target = Target::macos(&machine("arm"), "darwin", Architecture::Build("x86_64")).unwrap();
    assert_eq!(target.architectures(), ["x86_64"]);
    for archs in [
        &["arm64"][..],
        &["arm64", "arm64"],
        &["arm64", "noarch"],
        &["arm64", "../bad"],
    ] {
        assert!(Target::macos(&machine("arm"), "darwin", Architecture::Universal(archs)).is_err());
    }
}
#[test]
fn long_variant_names_use_upstream_ripemd160_fallback() {
    let port = port();
    let mut item = item(&port);
    item.variants = format!("+{}", "a".repeat(240));
    let target = Target::macos(&machine("arm"), "darwin", Architecture::Native).unwrap();
    // Independent OpenSSL RIPEMD-160 vector for '+' followed by 240 ASCII 'a'.
    assert_eq!(
        target.filename(&item).unwrap(),
        "curl-8.23.0_0+a6e20c40bf4b5b70f8a8fbbb28d660f8f916f62a.darwin_24.arm64.tbz2"
    );
    let mut long_port = port;
    long_port.name = "x".repeat(245);
    assert!(target.filename(&self::item(&long_port)).is_err());
}
#[test]
fn unresolved_or_invalid_identities_never_produce_download_requests() {
    let target = Target::macos(&machine("arm"), "darwin", Architecture::Native).unwrap();
    let port = port();
    let mut item = item(&port);
    item.variants_resolved = false;
    assert!(target.request(&item, 100).is_err());
    item.variants_resolved = true;
    for variant in ["-ssl", "+z+a", "+ssl/../../bad", "foo=bar", "@1"] {
        item.variants = variant.into();
        assert!(target.request(&item, 100).is_err());
    }
    for name in ["..", "../curl", "/curl", "curl\\bad", "curl\n"] {
        let mut port = self::port();
        port.name = name.into();
        assert!(target.request(&self::item(&port), 100).is_err());
    }
}
#[test]
fn url_components_cannot_change_mirror_host_or_path_structure() {
    let mut port = port();
    port.name = "app#fragment?query".into();
    let target = Target::macos(&machine("arm"), "darwin", Architecture::Native).unwrap();
    let request = target.request(&item(&port), 100).unwrap();
    let url = reqwest::Url::parse(&request.urls[0]).unwrap();
    assert_eq!(url.host_str(), Some("packages.macports.org"));
    assert!(url.query().is_none() && url.fragment().is_none());
    assert_eq!(url.path_segments().unwrap().count(), 2);
}

fn live_session(body: &str, variants: &str) -> port_rs::portfile::Session {
    let mut spec = port_rs::spec::PortSpec::named("app");
    if !variants.is_empty() {
        spec.parse_modifier(variants).unwrap();
    }
    port_rs::portfile::Session::open(&format!("name app; version 1; set os.arch arm; set configure.build_arch arm64; platforms darwin; {body}"),
        &BTreeMap::new(), &spec, port_rs::portfile::PortGroups::default()).unwrap()
}

#[test]
fn live_portfile_target_uses_selected_architectures_and_runtime_changes() {
    let mut session = live_session(
        "variant universal {}; set configure.universal_archs {x86_64 arm64}",
        "+universal",
    );
    let target = Target::from_session(&mut session, &machine("arm")).unwrap();
    assert_eq!(target.architectures(), ["arm64", "x86_64"]);
    assert_eq!(target.platform(), "darwin_24");
    session
        .invoke("set", &["configure.universal_archs", "arm64"])
        .unwrap();
    session
        .invoke("set", &["configure.build_arch", "x86_64"])
        .unwrap();
    let target = Target::from_session(&mut session, &machine("arm")).unwrap();
    assert_eq!(target.architectures(), ["x86_64"]);
    session
        .invoke("set", &["supported_archs", "noarch"])
        .unwrap();
    session.invoke("set", &["platforms", "any"]).unwrap();
    let target = Target::from_session(&mut session, &machine("arm")).unwrap();
    assert_eq!(target.architectures(), ["noarch"]);
    assert_eq!(target.platform(), "any_any");
}

#[test]
fn disabled_universal_and_platform_compatibility_come_from_live_portfile() {
    let mut session = live_session(
        "variant universal {}; default_variants +universal; set configure.universal_archs {x86_64 arm64}; platforms {darwin any}",
        "-universal",
    );
    let target = Target::from_session(&mut session, &machine("arm")).unwrap();
    assert_eq!(target.architectures(), ["arm64"]);
    assert_eq!(target.platform(), "darwin_any");
    // The architecture is the evaluated build target, not this machine's native
    // architecture; changing the machine only changes the default OS suffix.
    let target = Target::from_session(&mut session, &machine("i386")).unwrap();
    assert_eq!(target.architectures(), ["arm64"]);
}

#[test]
fn live_target_rejects_empty_duplicate_and_malformed_architectures() {
    for archs in [
        "",
        "arm64 arm64",
        "noarch arm64",
        "../escape",
        "{unterminated",
    ] {
        let mut session = live_session("", "");
        // Override the canonical procedure to exercise validation at the boundary
        // between the retained interpreter and archive coordinate construction.
        session.invoke("set", &["test_archs", archs]).unwrap();
        let body = "global test_archs; return $test_archs";
        session
            .invoke("proc", &["get_canonical_archs", "", body])
            .unwrap();
        assert!(
            Target::from_session(&mut session, &machine("arm")).is_err(),
            "{archs}"
        );
    }
}
