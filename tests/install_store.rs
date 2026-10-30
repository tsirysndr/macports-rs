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

fn registration() -> port_rs::registry::write::NewImage {
    use port_rs::registry::write::{ImageFile, NewImage};
    NewImage {
        name: "app".into(),
        version: "1".into(),
        variants: "+ssl".into(),
        archs: "arm64".into(),
        requested: true,
        requested_variants: "+ssl".into(),
        platform: "darwin".into(),
        os_major: "24".into(),
        files: vec![ImageFile {
            path: "/opt/local/app".into(),
            binary: false,
        }],
        dependencies: vec!["base".into()],
        ..NewImage::default()
    }
}

#[test]
fn registration_binds_identity_file_map_and_location_before_retaining_storage() {
    let software = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let mut writer =
        port_rs::registry::write::Writer::create(&database.path().join("registry.db")).unwrap();
    let port = port();
    let mut archive = stage(&bytes());
    for mismatch in 0..5 {
        let stored = store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default(),
            )
            .unwrap();
        let mut record = registration();
        match mismatch {
            0 => record.version = "2".into(),
            1 => record.archs = "x86_64".into(),
            2 => record.files.clear(),
            3 => record.location = "/wrong/archive.tbz2".into(),
            _ => record.variants.clear(),
        }
        assert!(stored.register(&mut writer, record).is_err());
        assert!(writer.transaction().unwrap().entries().unwrap().is_empty());
        assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    }
    let stored = store
        .stage(
            &mut archive,
            &item(&port),
            &target("arm"),
            Limits::default(),
        )
        .unwrap();
    let location = stored.location().to_owned();
    let signature = stored.signature_location().to_owned();
    let id = stored.register(&mut writer, registration()).unwrap();
    let tx = writer.transaction().unwrap();
    assert_eq!(tx.image_identity(id).unwrap().name, "app");
    assert!(!tx.entries().unwrap()[0].active);
    assert_eq!(fs::read(&location).unwrap(), bytes());
    assert_eq!(fs::read_to_string(signature).unwrap(), archive.signature());
    let db = rusqlite::Connection::open(database.path().join("registry.db")).unwrap();
    let registered: (String, i64) = db
        .query_row(
            "SELECT location,requested FROM ports WHERE id=?",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(registered, (location.to_str().unwrap().to_owned(), 1));
    assert_eq!(
        db.query_row("SELECT name FROM dependencies WHERE id=?", [id], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "base"
    );
}

#[test]
fn registration_update_and_commit_failures_remove_stored_files_and_registry_rows() {
    for commit_failure in [false, true] {
        let software = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        let db_path = database.path().join("registry.db");
        let store = Store::open(software.path()).unwrap();
        let mut writer = port_rs::registry::write::Writer::create(&db_path).unwrap();
        let db = rusqlite::Connection::open(&db_path).unwrap();
        if commit_failure {
            db.execute_batch("CREATE TABLE test_parent(id INTEGER PRIMARY KEY);
                CREATE TABLE test_child(id INTEGER REFERENCES test_parent(id) DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER fail_registration AFTER INSERT ON ports BEGIN INSERT INTO test_child VALUES(99); END;").unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER fail_registration BEFORE INSERT ON dependencies BEGIN SELECT RAISE(FAIL,'forced registration failure'); END;").unwrap();
        }
        let mut archive = stage(&bytes());
        let port = port();
        let stored = store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default(),
            )
            .unwrap();
        let error = stored.register(&mut writer, registration()).unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains(if commit_failure {
                "FOREIGN KEY"
            } else {
                "forced registration failure"
            }),
            "{text}"
        );
        assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
        for table in ["ports", "files", "dependencies"] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        db.execute_batch("DROP TRIGGER fail_registration").unwrap();
        store
            .stage(
                &mut archive,
                &item(&port),
                &target("arm"),
                Limits::default(),
            )
            .unwrap()
            .register(&mut writer, registration())
            .unwrap();
        assert_eq!(writer.transaction().unwrap().entries().unwrap().len(), 1);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn stored_registered_archive_can_be_prepared_and_activated_end_to_end() {
    use port_rs::install::{activation::Plan, extract};
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let c = contents(&["opt/local/app"]);
    let bytes = compress(&tar_with_owner(
        &[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("opt/local/app", b'0', b"payload", ""),
        ],
        unsafe { libc::geteuid() }.into(),
        unsafe { libc::getegid() }.into(),
    ));
    let mut archive = stage(&bytes);
    let port = port();
    let item = item(&port);
    let target = target("arm");
    let mut writer =
        port_rs::registry::write::Writer::create(&work.path().join("registry.db")).unwrap();
    let stored = store
        .stage(&mut archive, &item, &target, Limits::default())
        .unwrap();
    let location = stored.location().to_owned();
    let prepared =
        extract::prepare_for_target(&mut archive, &item, &target, work.path(), Limits::default())
            .unwrap();
    let mut record = registration();
    record.files = prepared.registry_files().unwrap();
    let id = stored.register(&mut writer, record).unwrap();
    let plan = Plan::inspect(&prepared, root.path()).unwrap();
    plan.activate(&mut writer, id).unwrap();
    drop(prepared);
    assert_eq!(
        fs::read(root.path().join("opt/local/app")).unwrap(),
        b"payload"
    );
    assert_eq!(fs::read(location).unwrap(), bytes);
    assert!(writer.transaction().unwrap().entries().unwrap()[0].active);
}

#[test]
fn reload_authenticates_private_bytes_and_rejects_corrupt_or_oversized_cache() {
    use port_rs::install::store::LoadLimits;
    use std::io::Read;
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let port = port();
    let item = item(&port);
    let target = target("arm");
    let original = bytes();
    let mut archive = stage(&original);
    let stored = store
        .stage(&mut archive, &item, &target, Limits::default())
        .unwrap();
    let location = stored.location().to_owned();
    let signature = stored.signature_location().to_owned();
    stored.persist();
    let limits = || LoadLimits {
        archive_bytes: 1024 * 1024,
        inspection: Limits::default(),
    };
    let keys = [trusted_key()];
    let mut loaded = store
        .load(&item, &target, &keys, work.path(), limits())
        .unwrap();
    assert_eq!(loaded.verification(), archive.verification());
    assert!(
        store
            .load(&item, &target, &[], work.path(), limits())
            .is_err()
    );
    assert!(
        store
            .load(
                &item,
                &target,
                &keys,
                work.path(),
                LoadLimits {
                    archive_bytes: 1,
                    inspection: Limits::default()
                }
            )
            .is_err()
    );
    fs::write(&location, b"tampered").unwrap();
    assert!(
        store
            .load(&item, &target, &keys, work.path(), limits())
            .is_err()
    );
    let mut retained = Vec::new();
    loaded.reader().unwrap().read_to_end(&mut retained).unwrap();
    assert_eq!(retained, original);
    assert_eq!(fs::read(&location).unwrap(), b"tampered");
    fs::write(&location, &original).unwrap();
    fs::write(&signature, "x".repeat(4097)).unwrap();
    assert!(
        store
            .load(&item, &target, &keys, work.path(), limits())
            .err()
            .unwrap()
            .to_string()
            .contains("signature exceeds")
    );
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
}

#[test]
fn reload_rejects_links_special_files_and_authentic_but_wrong_identity() {
    use port_rs::install::store::LoadLimits;
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let port = port();
    let item = item(&port);
    let target = target("arm");
    let mut archive = stage(&bytes());
    let stored = store
        .stage(&mut archive, &item, &target, Limits::default())
        .unwrap();
    let location = stored.location().to_owned();
    let signature = stored.signature_location().to_owned();
    stored.persist();
    let load = || {
        store.load(
            &item,
            &target,
            &[trusted_key()],
            work.path(),
            LoadLimits {
                archive_bytes: 1024 * 1024,
                inspection: Limits::default(),
            },
        )
    };
    let saved = location.with_extension("saved");
    fs::rename(&location, &saved).unwrap();
    symlink(&saved, &location).unwrap();
    assert!(load().is_err());
    fs::remove_file(&location).unwrap();
    // Opening a FIFO must reject immediately, even without any writer.
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(location.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(load().is_err());
    fs::remove_file(&location).unwrap();
    fs::rename(&saved, &location).unwrap();
    let c = contents(&["opt/local/app"])
        .replace("app-1_0", "app-2_0")
        .replace("@portversion 1", "@portversion 2");
    let wrong = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/app", b'0', b"payload", ""),
    ]));
    let signed_wrong = stage(&wrong);
    fs::write(&location, &wrong).unwrap();
    fs::write(&signature, signed_wrong.signature()).unwrap();
    assert!(load().err().unwrap().to_string().contains("does not match"));
    fs::rename(software.path().join("app"), software.path().join("moved")).unwrap();
    symlink("moved", software.path().join("app")).unwrap();
    assert!(load().is_err());
}

#[test]
fn prepared_inspection_reuse_is_bound_to_authenticated_bytes() {
    use port_rs::install::extract;
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let port = port();
    let item = item(&port);
    let target = target("arm");
    let mut archive = stage(&bytes());
    let prepared =
        extract::prepare_for_target(&mut archive, &item, &target, work.path(), Limits::default())
            .unwrap();
    let c = contents(&["opt/local/app"]);
    let different = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/app", b'0', b"different payload", ""),
    ]));
    let mut other = stage(&different);
    assert!(
        store
            .stage_prepared(&mut other, &item, &target, &prepared)
            .err()
            .unwrap()
            .to_string()
            .contains("authenticated archive bytes")
    );
    assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    let stored = store
        .stage_prepared(&mut archive, &item, &target, &prepared)
        .unwrap();
    // Both views share the same immutable inventory instead of cloning entries.
    assert!(std::ptr::eq(stored.inspection(), prepared.inspection()));
    drop(prepared);
    assert_eq!(stored.inspection().contents.name, "app");
    assert_eq!(fs::read(stored.location()).unwrap(), bytes());
    stored.rollback().unwrap();
}
