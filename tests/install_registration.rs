use port_rs::{
    index::Port,
    install::{
        Action, Item,
        registration::{self, Inputs},
    },
    portfile::{PortGroups, Session},
    source::Platform,
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn spec() -> PortSpec {
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("+ssl-docs").unwrap();
    spec
}
fn session(spec: &PortSpec) -> Session {
    Session::open("name app; version 1; revision 2; epoch 3; variant ssl {}; variant docs {}; platforms {darwin any}; set configure.build_arch arm64; set os.arch arm; set configure.cxx_stdlib libc++; set cxx_stdlib libc++",
        &BTreeMap::new(), spec, PortGroups::default()).unwrap()
}
fn port() -> Port {
    Port {
        name: "app".into(),
        fields: BTreeMap::from([
            ("version".into(), "1".into()),
            ("revision".into(), "2".into()),
            ("epoch".into(), "3".into()),
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
fn machine() -> Platform {
    Platform {
        darwin_major: 24,
        architecture: "arm",
    }
}
#[test]
fn builds_registration_from_live_properties_and_resolved_inputs() {
    let spec = spec();
    let mut session = session(&spec);
    let port = port();
    let (target, record) = registration::from_session(
        &mut session,
        &item(&port),
        &machine(),
        Inputs {
            requested: &spec,
            dependencies: vec!["openssl3".into()],
            portfile: "retained Portfile source".into(),
        },
    )
    .unwrap();
    assert_eq!(
        (
            record.name.as_str(),
            record.version.as_str(),
            record.revision,
            record.epoch
        ),
        ("app", "1", 2, 3)
    );
    assert_eq!(record.variants, "+ssl");
    assert_eq!(record.requested_variants, "+ssl-docs");
    assert!(record.requested);
    assert_eq!(
        (
            record.platform.as_str(),
            record.os_major.as_str(),
            record.archs.as_str()
        ),
        ("darwin", "any", "arm64")
    );
    assert_eq!(target.platform(), "darwin_any");
    assert_eq!(record.dependencies, ["openssl3"]);
    assert_eq!(record.portfile, "retained Portfile source");
    assert!(record.cxx_stdlib.is_empty());
    assert!(!record.cxx_stdlib_overridden);
    assert!(record.files.is_empty());
    assert!(record.location.is_empty());
    session
        .invoke("set", &["configure.cxx_stdlib", "libstdc++"])
        .unwrap();
    session
        .invoke("set", &["supported_archs", "noarch"])
        .unwrap();
    session.invoke("set", &["platforms", "any"]).unwrap();
    let (_, record) = registration::from_session(
        &mut session,
        &item(&port),
        &machine(),
        Inputs {
            requested: &spec,
            dependencies: vec![],
            portfile: String::new(),
        },
    )
    .unwrap();
    assert!(record.cxx_stdlib_overridden);
    assert_eq!(
        (
            record.platform.as_str(),
            record.os_major.as_str(),
            record.archs.as_str()
        ),
        ("any", "any", "noarch")
    );
}
#[test]
fn rejects_stale_plans_and_unresolved_providers() {
    let spec = spec();
    let port = port();
    for (key, value) in [
        ("version", "2"),
        ("revision", "4"),
        ("epoch", "4"),
        ("revision", "-1"),
        ("subport", "other"),
    ] {
        let mut session = session(&spec);
        session.invoke("set", &[key, value]).unwrap();
        assert!(
            registration::from_session(
                &mut session,
                &item(&port),
                &machine(),
                Inputs {
                    requested: &spec,
                    dependencies: vec![],
                    portfile: String::new()
                }
            )
            .is_err()
        );
    }
    let mut session = session(&spec);
    let mut unresolved = item(&port);
    unresolved
        .provider_fallbacks
        .push("bin:openssl:openssl".into());
    assert!(
        registration::from_session(
            &mut session,
            &unresolved,
            &machine(),
            Inputs {
                requested: &spec,
                dependencies: vec![],
                portfile: String::new()
            }
        )
        .is_err()
    );
    unresolved.provider_fallbacks.clear();
    unresolved.variants.clear();
    assert!(
        registration::from_session(
            &mut session,
            &unresolved,
            &machine(),
            Inputs {
                requested: &spec,
                dependencies: vec![],
                portfile: String::new()
            }
        )
        .is_err()
    );
}

#[test]
fn resolves_registration_providers_using_live_session_context() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("provided"), b"provider").unwrap();
    let spec = spec();
    let port = port();
    let mut session = session(&spec);
    for (key, value) in [
        ("prefix", root.path().to_str().unwrap()),
        ("frameworks_dir", root.path().to_str().unwrap()),
        ("os.platform", "darwin"),
        ("depends_lib", "path:provided:fallback port:required"),
        ("depends_run", "port:REQUIRED"),
        ("depends_build", "port:build-only"),
    ] {
        session.invoke("set", &[key, value]).unwrap();
    }
    let (_, record) = registration::with_providers(
        &mut session,
        &item(&port),
        &machine(),
        &spec,
        "retained".into(),
        None,
    )
    .unwrap();
    assert_eq!(record.dependencies, ["required"]);
    assert_eq!(record.portfile, "retained");
    session
        .invoke(
            "set",
            &["prefix", root.path().join("other").to_str().unwrap()],
        )
        .unwrap();
    let (_, record) = registration::with_providers(
        &mut session,
        &item(&port),
        &machine(),
        &spec,
        String::new(),
        None,
    )
    .unwrap();
    assert_eq!(record.dependencies, ["fallback", "required"]);
}
