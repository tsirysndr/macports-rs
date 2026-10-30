use port_rs::{
    dependencies::provider::{Resolver, Search},
    registry::{
        Registry,
        write::{ActivationFile, ImageFile, NewImage, Writer},
    },
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn search(root: &Path) -> Search {
    Search {
        prefix: root.into(),
        frameworks_dir: root.join("Frameworks"),
        platform: "darwin".into(),
        environment: BTreeMap::from([(
            "PATH".into(),
            format!(
                "{}:{}",
                root.join("first").display(),
                root.join("second").display()
            ),
        )]),
    }
}
#[test]
fn probes_literal_paths_executables_and_versioned_libraries() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["first", "second", "lib", "Frameworks"] {
        fs::create_dir(root.path().join(dir)).unwrap();
    }
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("bin:tool:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("first/tool"), b"not executable").unwrap();
    fs::set_permissions(
        root.path().join("first/tool"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(
        resolver.resolve("bin:tool:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("second/tool"), b"not actually executed").unwrap();
    fs::set_permissions(
        root.path().join("second/tool"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(resolver.resolve("bin:tool:fallback").unwrap(), None);
    symlink("missing", root.path().join("lib/dangling")).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/dangling:fallback").unwrap(),
        None
    );
    assert_eq!(resolver.resolve("path:lib:fallback").unwrap(), None);
    assert_eq!(
        resolver.resolve("path:lib/not*:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("lib/libexample.2.dylib"), b"library").unwrap();
    assert_eq!(resolver.resolve("lib:libexample.2:fallback").unwrap(), None);
    assert_eq!(
        resolver.resolve("lib:libexample.3:fallback").unwrap(),
        Some("fallback".into())
    );
    assert_eq!(
        resolver.resolve("port:fallback").unwrap(),
        Some("fallback".into())
    );
}
fn register(writer: &mut Writer, name: &str, actual: Option<&Path>) {
    let mut tx = writer.transaction().unwrap();
    let id = tx
        .register(&NewImage {
            name: name.into(),
            version: "1".into(),
            location: format!("/images/{name}.tbz2"),
            files: actual
                .map(|_| {
                    vec![ImageFile {
                        path: "/payload".into(),
                        binary: false,
                    }]
                })
                .unwrap_or_default(),
            ..NewImage::default()
        })
        .unwrap();
    let files = actual
        .map(|path| {
            vec![ActivationFile {
                path: "/payload".into(),
                actual_path: path.to_str().unwrap().into(),
            }]
        })
        .unwrap_or_default();
    tx.activate(id, &files).unwrap();
    tx.commit().unwrap();
}
#[test]
fn registry_provider_and_active_fallback_follow_upstream_priority() {
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    let path = root.path().join("lib/actual");
    fs::write(&path, b"provided").unwrap();
    let path = path.canonicalize().unwrap();
    let dbpath = database.path().join("registry.db");
    let mut writer = Writer::create(&dbpath).unwrap();
    register(&mut writer, "actual-provider", Some(&path));
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/actual:fallback").unwrap(),
        Some("actual-provider".into())
    );
    drop(resolver);
    drop(registry);
    register(&mut writer, "fallback", None);
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/actual:FALLBACK").unwrap(),
        Some("FALLBACK".into())
    );
}
#[test]
fn library_environment_paths_preserve_upstream_order_and_path_list_semantics() {
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("literal:directory");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("libunique.1.dylib"), b"library").unwrap();
    let mut context = search(root.path());
    context
        .environment
        .insert("DYLD_LIBRARY_PATH".into(), library.to_str().unwrap().into());
    assert_eq!(
        Resolver::new(None, context)
            .unwrap()
            .resolve("lib:libunique.1:fallback")
            .unwrap(),
        None
    );
    let mut context = search(root.path());
    context.platform = "freebsd".into();
    context
        .environment
        .insert("DYLD_LIBRARY_PATH".into(), library.to_str().unwrap().into());
    fs::write(library.join("libunique.so.1"), b"library").unwrap();
    assert_eq!(
        Resolver::new(None, context)
            .unwrap()
            .resolve("lib:libunique.1:fallback")
            .unwrap(),
        None
    );
}

#[test]
fn runtime_registration_dependencies_use_live_lists_and_omit_system_providers() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("system-provider"), b"present").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let mut session = port_rs::portfile::Session::open(
        "name app; version 1; depends_lib port:first path:system-provider:unneeded; depends_run port:FIRST port:second; depends_build port:build-only",
        &BTreeMap::new(), &port_rs::spec::PortSpec::named("app"), port_rs::portfile::PortGroups::default()).unwrap();
    assert_eq!(
        resolver.runtime_dependencies(&mut session).unwrap(),
        ["first", "second"]
    );
    session
        .invoke("set", &["depends_run", "port:third"])
        .unwrap();
    assert_eq!(
        resolver.runtime_dependencies(&mut session).unwrap(),
        ["first", "third"]
    );
}

#[test]
fn provider_evidence_distinguishes_missing_fallback_from_system_and_active_ports() {
    use port_rs::dependencies::provider::Resolution;
    let root = tempfile::tempdir().unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let missing = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(missing, Resolution::Missing("fallback".into()));
    assert!(!missing.is_satisfied());
    fs::write(root.path().join("provided"), b"system").unwrap();
    let present = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(present, Resolution::System(root.path().join("provided")));
    assert!(present.is_satisfied());
    let database = tempfile::tempdir().unwrap();
    let dbpath = database.path().join("registry.db");
    let mut writer = Writer::create(&dbpath).unwrap();
    register(&mut writer, "fallback", None);
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    let active = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(active, Resolution::ActivePort("fallback".into()));
    assert!(active.is_satisfied());
}

#[test]
fn planning_skips_satisfied_providers_but_preserves_explicit_requests_and_missing_fallbacks() {
    use port_rs::{index::Index, install, spec::PortSpec};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("provided"), b"system").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let index_text = |fallback: bool| {
        let app = "name app version 1 revision 0 depends_run path:provided:fallback";
        let mut text = format!("app {}\n{app}\n", app.len() + 1);
        if fallback {
            let body = "name fallback version 1 revision 0";
            text.push_str(&format!("fallback {}\n{body}\n", body.len() + 1));
        }
        Index::parse(&text).unwrap()
    };
    let index = index_text(false);
    // A satisfied provider doesn't even require the fallback in the index.
    let plan =
        install::plan_with_providers(&index, &[PortSpec::named("app")], &[], &resolver).unwrap();
    assert_eq!(plan.len(), 1);
    assert!(plan[0].provider_fallbacks.is_empty());
    let index = index_text(true);
    let plan = install::plan_with_providers(
        &index,
        &[PortSpec::named("app"), PortSpec::named("fallback")],
        &[],
        &resolver,
    )
    .unwrap();
    assert_eq!(
        plan.iter()
            .map(|item| item.port.name.as_str())
            .collect::<Vec<_>>(),
        ["fallback", "app"]
    );
    fs::remove_file(root.path().join("provided")).unwrap();
    let plan =
        install::plan_with_providers(&index, &[PortSpec::named("app")], &[], &resolver).unwrap();
    assert_eq!(
        plan.iter()
            .map(|item| item.port.name.as_str())
            .collect::<Vec<_>>(),
        ["fallback", "app"]
    );
    assert!(plan.iter().all(|item| item.provider_fallbacks.is_empty()));
}
