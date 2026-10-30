use port_rs::registry::{
    self, Registry,
    write::{ActivationFile, ImageFile, NewImage, Writer},
};
use rusqlite::Connection;
use std::os::unix::fs::PermissionsExt;
fn image(name: &str) -> NewImage {
    NewImage {
        name: name.into(),
        version: "1.0".into(),
        revision: 1,
        epoch: 0,
        variants: "+ssl".into(),
        requested_variants: "+ssl-universal".into(),
        requested: true,
        location: format!("/opt/local/var/macports/software/{name}/{name}.tbz2"),
        portfile: "PortSystem 1.0\n# recorded as data; never executed\n".into(),
        platform: "darwin".into(),
        os_major: "24".into(),
        archs: "arm64".into(),
        cxx_stdlib: "libc++".into(),
        files: vec![ImageFile {
            path: format!("/opt/local/bin/{name}"),
            binary: true,
        }],
        dependencies: vec!["zlib".into(), "ZLIB".into()],
        ..NewImage::default()
    }
}
fn mapping(name: &str) -> Vec<ActivationFile> {
    vec![ActivationFile {
        path: format!("/opt/local/bin/{name}"),
        actual_path: format!("/opt/local/bin/{name}"),
    }]
}
#[test]
fn initializes_native_schema_atomically_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let writer = Writer::create(&path).unwrap();
    assert!(registry::read(Some(&path)).unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM metadata WHERE key='version'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "1.215"
    );
    assert_eq!(
        db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    for table in [
        "portgroups",
        "snapshots",
        "snapshot_ports",
        "snapshot_files",
    ] {
        db.prepare(&format!("SELECT * FROM {table}")).unwrap();
    }
    assert!(Writer::create(&path).is_err());
    drop(db);
    drop(writer);
    let other = temp.path().join("existing");
    std::fs::write(&other, b"preserve").unwrap();
    assert!(Writer::create(&other).is_err());
    assert_eq!(std::fs::read(other).unwrap(), b"preserve");
}
#[test]
fn registration_roundtrips_through_native_readers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx.register(&image("curl")).unwrap();
    tx.commit().unwrap();
    let reader = Registry::open(Some(&path)).unwrap().unwrap();
    let entries = reader.entries().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.id, id);
    assert_eq!(entry.full_version(), "1.0_1+ssl");
    assert!(!entry.active);
    assert!(entry.requested);
    assert_eq!(entry.platform, "darwin");
    assert_eq!(entry.os_major, "24");
    assert_eq!(reader.files(entry).unwrap(), ["/opt/local/bin/curl"]);
    assert_eq!(reader.dependent_edges().unwrap(), [("zlib".into(), id)]);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT portfile FROM ports", [], |r| r.get::<_, String>(0))
            .unwrap(),
        image("curl").portfile
    );
}
#[test]
fn dropped_transactions_rollback_all_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    {
        let mut tx = writer.transaction().unwrap();
        let id = tx.register(&image("curl")).unwrap();
        tx.activate(id, &mapping("curl")).unwrap();
    }
    assert!(registry::read(Some(&path)).unwrap().is_empty());
    let db = Connection::open(&path).unwrap();
    for table in ["files", "dependencies"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn failed_registration_rolls_back_to_savepoint() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_bad_dep BEFORE INSERT ON dependencies WHEN NEW.name='bad' BEGIN SELECT RAISE(FAIL,'test failure'); END;").unwrap();
    let mut tx = writer.transaction().unwrap();
    tx.register(&image("good")).unwrap();
    let mut bad = image("badimage");
    bad.dependencies.push("bad".into());
    assert!(tx.register(&bad).is_err());
    tx.commit().unwrap();
    assert_eq!(
        registry::read(Some(&path))
            .unwrap()
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["good"]
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn activation_renames_file_maps_and_deactivation_retains_images() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx.register(&image("curl")).unwrap();
    tx.activate(
        id,
        &[ActivationFile {
            path: "/opt/local/bin/curl".into(),
            actual_path: "/opt/local/bin/curl-custom".into(),
        }],
    )
    .unwrap();
    tx.commit().unwrap();
    {
        let reader = Registry::open(Some(&path)).unwrap().unwrap();
        let entries = reader.entries().unwrap();
        assert!(entries[0].active);
        assert_eq!(
            reader.files(&entries[0]).unwrap(),
            ["/opt/local/bin/curl-custom"]
        );
        assert_eq!(
            reader.owner("/opt/local/bin/curl-custom", true).unwrap(),
            Some("curl".into())
        );
    }
    let mut tx = writer.transaction().unwrap();
    assert!(tx.remove(id).is_err());
    tx.deactivate(id).unwrap();
    tx.set_requested(id, false).unwrap();
    tx.commit().unwrap();
    {
        let reader = Registry::open(Some(&path)).unwrap().unwrap();
        let entries = reader.entries().unwrap();
        assert!(!entries[0].active);
        assert!(!entries[0].requested);
        assert_eq!(reader.files(&entries[0]).unwrap(), ["/opt/local/bin/curl"]);
        assert!(
            reader
                .owner("/opt/local/bin/curl-custom", true)
                .unwrap()
                .is_none()
        );
    }
    let mut tx = writer.transaction().unwrap();
    tx.remove(id).unwrap();
    tx.commit().unwrap();
    let db = Connection::open(&path).unwrap();
    for table in ["ports", "files", "dependencies"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn activation_rejects_incomplete_maps_and_ownership_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let first = tx.register(&image("first")).unwrap();
    let second = tx.register(&image("second")).unwrap();
    assert!(tx.activate(first, &[]).is_err());
    tx.activate(first, &mapping("first")).unwrap();
    assert!(
        tx.activate(
            second,
            &[ActivationFile {
                path: "/opt/local/bin/second".into(),
                actual_path: "/OPT/LOCAL/BIN/FIRST".into()
            }]
        )
        .is_err()
    );
    tx.commit().unwrap();
    let entries = registry::read(Some(&path)).unwrap();
    assert!(entries.iter().find(|e| e.id == first).unwrap().active);
    assert!(!entries.iter().find(|e| e.id == second).unwrap().active);
}
#[test]
fn multiple_versions_can_be_registered_but_only_one_activated() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let first = tx.register(&image("curl")).unwrap();
    let mut newer = image("CURL");
    newer.version = "2.0".into();
    let second = tx.register(&newer).unwrap();
    tx.activate(first, &mapping("curl")).unwrap();
    assert!(tx.activate(second, &mapping("CURL")).is_err());
    assert!(tx.register(&image("curl")).is_err());
    tx.commit().unwrap();
    assert_eq!(registry::read(Some(&path)).unwrap().len(), 2);
}
#[test]
fn validation_rejects_unsafe_paths_and_noncanonical_variants() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    for file in [
        "relative",
        "/opt/../outside",
        "/opt//file",
        "/opt/./file",
        "/",
    ] {
        let mut bad = image("app");
        bad.files[0].path = file.into();
        assert!(tx.register(&bad).is_err());
    }
    for variants in ["+z+a", "-ssl", "+ssl+ssl", "@1"] {
        let mut bad = image("app");
        bad.variants = variants.into();
        assert!(tx.register(&bad).is_err());
    }
    let mut bad = image("app");
    bad.files.push(ImageFile {
        path: "/OPT/LOCAL/BIN/APP".into(),
        binary: false,
    });
    assert!(tx.register(&bad).is_err());
    tx.commit().unwrap();
    assert!(registry::read(Some(&path)).unwrap().is_empty());
}
#[test]
fn writers_refuse_missing_old_and_future_registries_without_migration() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    assert!(Writer::open(&missing).is_err());
    assert!(!missing.exists());
    for version in ["1.214", "9.0"] {
        let path = temp.path().join(version);
        drop(Writer::create(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE metadata SET value=?1 WHERE key='version'",
            [version],
        )
        .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(Writer::open(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
#[test]
fn wal_readers_keep_a_snapshot_while_writer_commits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let reader = Registry::open(Some(&path)).unwrap().unwrap();
    assert!(reader.entries().unwrap().is_empty());
    let mut tx = writer.transaction().unwrap();
    tx.register(&image("app")).unwrap();
    tx.commit().unwrap();
    assert!(reader.entries().unwrap().is_empty());
    assert_eq!(registry::read(Some(&path)).unwrap().len(), 1);
}

#[test]
fn writer_accepts_symlinked_parent_but_rejects_symlinked_database() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    symlink(&real, &alias).unwrap();
    let path = alias.join("registry.db");
    drop(Writer::create(&path).unwrap());
    assert!(Writer::open(&path).is_ok());
    let link = real.join("linked.db");
    symlink(real.join("registry.db"), &link).unwrap();
    assert!(Writer::open(&link).is_err());
}

#[test]
fn activation_preflight_is_read_only_and_activation_rechecks_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let first = tx.register(&image("first")).unwrap();
    let second = tx.register(&image("second")).unwrap();
    tx.check_activation(first, &mapping("first")).unwrap();
    assert!(
        !tx.entries()
            .unwrap()
            .iter()
            .find(|entry| entry.id == first)
            .unwrap()
            .active
    );
    assert!(tx.check_activation(first, &[]).is_err());
    assert!(tx.check_activation(i64::MAX, &mapping("first")).is_err());
    tx.activate(
        second,
        &[ActivationFile {
            path: "/opt/local/bin/second".into(),
            actual_path: "/OPT/LOCAL/BIN/FIRST".into(),
        }],
    )
    .unwrap();
    assert!(tx.check_activation(first, &mapping("first")).is_err());
    assert!(tx.activate(first, &mapping("first")).is_err());
    tx.commit().unwrap();
    let entries = registry::read(Some(&path)).unwrap();
    assert!(
        !entries
            .iter()
            .find(|entry| entry.id == first)
            .unwrap()
            .active
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry.id == second)
            .unwrap()
            .active
    );
}

#[test]
fn preflight_rejects_duplicate_rows_in_a_damaged_image_map() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx.register(&image("app")).unwrap();
    tx.commit().unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute("INSERT INTO files SELECT * FROM files WHERE id=?1", [id])
        .unwrap();
    let tx = writer.transaction().unwrap();
    assert!(
        tx.check_activation(id, &mapping("app"))
            .unwrap_err()
            .to_string()
            .contains("duplicate registered file map")
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM files WHERE active", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_activation_after_preflight_rolls_back_all_file_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_activation BEFORE UPDATE OF active ON files WHEN NEW.path='/second' AND NEW.active=1 BEGIN SELECT RAISE(FAIL,'test failure'); END;").unwrap();
    let mut tx = writer.transaction().unwrap();
    let mut record = image("app");
    record.files.push(ImageFile {
        path: "/second".into(),
        binary: false,
    });
    let id = tx.register(&record).unwrap();
    let mut files = mapping("app");
    files.push(ActivationFile {
        path: "/second".into(),
        actual_path: "/second".into(),
    });
    tx.check_activation(id, &files).unwrap();
    assert!(tx.activate(id, &files).is_err());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM files WHERE active OR actual_path IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(!registry::read(Some(&path)).unwrap()[0].active);
}
