#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::{
    index::Port,
    install::{
        Action, Item,
        inspect::Limits,
        store::Store,
        target::{Architecture, Target},
    },
    source::Platform,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink};
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
fn target(arch: &'static str) -> Target {
    Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: arch,
        },
        "darwin",
        Architecture::Native,
    )
    .unwrap()
}
fn bytes() -> Vec<u8> {
    let c = contents(&["opt/local/app"]);
    compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/app", b'0', b"payload", ""),
    ]))
}
#[test]
fn stores_exact_verified_bytes_and_signature_with_rollback_and_retention() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let bytes = bytes();
    let mut archive = stage(&bytes);
    let signature = archive.signature().to_owned();
    let port = port();
    let stored = store
        .stage(
            &mut archive,
            &item(&port),
            &target("arm"),
            Limits::default(),
        )
        .unwrap();
    assert_eq!(
        stored.location(),
        temp.path()
            .join("app/app-1_0+ssl.darwin_24.arm64.tbz2")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(fs::read(stored.location()).unwrap(), bytes);
    assert_eq!(
        fs::read_to_string(stored.signature_location()).unwrap(),
        signature
    );
    assert_eq!(stored.inspection().contents.name, "app");
    drop(stored);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let stored = store
        .stage(
            &mut archive,
            &item(&port),
            &target("arm"),
            Limits::default(),
        )
        .unwrap();
    let location = stored.location().to_owned();
    let signature_location = stored.signature_location().to_owned();
    stored.persist();
    assert_eq!(fs::read(&location).unwrap(), bytes);
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(fs::read(location).unwrap(), bytes);
    assert_eq!(fs::read_to_string(signature_location).unwrap(), signature);
}
#[test]
fn failed_signature_publication_removes_archive_and_preserves_existing_entries() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("app")).unwrap();
    let signature = temp.path().join("app/app-1_0+ssl.darwin_24.arm64.tbz2.sig");
    symlink("missing", &signature).unwrap();
    let store = Store::open(temp.path()).unwrap();
    let mut archive = stage(&bytes());
    let port = port();
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("app")).unwrap().count(), 1);
    assert_eq!(
        fs::read_link(&signature).unwrap(),
        std::path::Path::new("missing")
    );
    fs::remove_file(signature).unwrap();
    store
        .stage(
            &mut archive,
            &item(&port),
            &target("arm"),
            Limits::default(),
        )
        .unwrap()
        .rollback()
        .unwrap();
    assert!(temp.path().join("app").is_dir());
    assert_eq!(fs::read_dir(temp.path().join("app")).unwrap().count(), 0);
}
#[test]
fn validates_identity_architecture_and_physical_directories_before_storage() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let mut archive = stage(&bytes());
    let mut port = port();
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("i386"),
                Limits::default()
            )
            .is_err()
    );
    port.fields.insert("version".into(), "2".into());
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default()
            )
            .is_err()
    );
    port.name = "../escape".into();
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    port.name = "app".into();
    port.fields.insert("version".into(), "1".into());
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), temp.path().join("app")).unwrap();
    assert!(
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default()
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(Store::open(&temp.path().join("app/")).is_err());
}
