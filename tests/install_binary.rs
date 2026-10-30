#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::{
    index::Port,
    install::{
        Action, Item,
        binary::Installer,
        inspect::Limits,
        store::Store,
        target::{Architecture, Target},
    },
    progress::Progress,
    registry::write::{NewImage, Writer},
    source::Platform,
};
use std::{collections::BTreeMap, fs};
fn port() -> Port {
    Port {
        name: "app".into(),
        fields: BTreeMap::from([
            ("version".into(), "1".into()),
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
fn target() -> Target {
    Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        "darwin",
        Architecture::Native,
    )
    .unwrap()
}
fn record() -> NewImage {
    NewImage {
        name: "app".into(),
        version: "1".into(),
        variants: "+ssl".into(),
        archs: "arm64".into(),
        dependencies: vec!["base".into()],
        ..NewImage::default()
    }
}
fn archive() -> port_rs::install::archive::VerifiedArchive {
    let c = contents(&["opt/local/app", "opt/local/link"]);
    stage(&compress(&tar_with_owner(
        &[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("opt/local/app", b'0', &0xfeedfacfu32.to_ne_bytes(), ""),
            ("opt/local/link", b'2', b"", "app"),
        ],
        unsafe { libc::geteuid() }.into(),
        unsafe { libc::getegid() }.into(),
    )))
}
#[test]
fn installs_verified_binary_with_generated_inventory_and_requested_status() {
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    let path = database.path().join("registry.db");
    let store = Store::open(software.path()).unwrap();
    let mut registry = Writer::create(&path).unwrap();
    let progress = Progress::new(true);
    let port = port();
    let installed = Installer {
        store: &store,
        registry: &mut registry,
        staging: work.path(),
        root: root.path(),
        limits: Limits::default(),
        progress: &progress,
    }
    .install(&mut archive(), &item(&port), &target(), record())
    .unwrap();
    assert!(installed.location.is_file());
    assert_eq!(
        fs::read(root.path().join("opt/local/app")).unwrap(),
        0xfeedfacfu32.to_ne_bytes()
    );
    assert_eq!(
        fs::read_link(root.path().join("opt/local/link")).unwrap(),
        std::path::Path::new("app")
    );
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state,requested FROM ports WHERE id=?",
            [installed.id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("installed".into(), 1)
    );
    assert_eq!(
        db.query_row(
            "SELECT binary FROM files WHERE path='/opt/local/app'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT binary FROM files WHERE path='/opt/local/link'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn rejects_unresolved_plans_before_mutation_and_retains_registered_image_on_activation_conflict() {
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let mut registry = Writer::create(&database.path().join("registry.db")).unwrap();
    let progress = Progress::new(true);
    let port = port();
    let mut installer = Installer {
        store: &store,
        registry: &mut registry,
        staging: work.path(),
        root: root.path(),
        limits: Limits::default(),
        progress: &progress,
    };
    for step in [Action::Activate, Action::AlreadyActive, Action::Resolve] {
        let mut item = item(&port);
        item.action = step;
        assert!(
            installer
                .install(&mut archive(), &item, &target(), record())
                .is_err()
        );
    }
    let mut unresolved = item(&port);
    unresolved
        .provider_fallbacks
        .push("path:lib/app:app".into());
    assert!(
        installer
            .install(&mut archive(), &unresolved, &target(), record())
            .is_err()
    );
    unresolved.provider_fallbacks.clear();
    unresolved.variants_resolved = false;
    assert!(
        installer
            .install(&mut archive(), &unresolved, &target(), record())
            .is_err()
    );
    assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let mut wrong = record();
    wrong.version = "2".into();
    assert!(
        installer
            .install(&mut archive(), &item(&port), &target(), wrong)
            .is_err()
    );
    assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    assert!(
        installer
            .registry
            .transaction()
            .unwrap()
            .entries()
            .unwrap()
            .is_empty()
    );
    fs::create_dir_all(root.path().join("opt/local")).unwrap();
    fs::write(root.path().join("opt/local/app"), b"existing").unwrap();
    let error = installer
        .install(&mut archive(), &item(&port), &target(), record())
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("retained for retry"));
    assert_eq!(
        fs::read(root.path().join("opt/local/app")).unwrap(),
        b"existing"
    );
    assert!(!root.path().join("opt/local/link").exists());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let entries = registry.transaction().unwrap().entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].active);
    assert_eq!(
        fs::read_dir(software.path().join("app")).unwrap().count(),
        2
    );
}
