#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::{
    index::Index,
    install::{
        self,
        activation::{ConflictKind, Plan},
        extract,
        inspect::Limits,
    },
    spec::PortSpec,
};
use std::{fs, os::unix::fs::symlink};
fn image(directory: &std::path::Path) -> extract::PreparedImage {
    let body = "name app version 1 revision 0 variants ssl vinfo {ssl {is_default +}}";
    let index = Index::parse(&format!("app {}\n{body}\n", body.len() + 1)).unwrap();
    let items = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let c = contents(&[
        "opt/local/bin/app",
        "opt/local/bin/alias",
        "opt/local/bin/hard",
    ]);
    let bytes = compress(&tar_with_owner(
        &[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("+PORTFILE", b'0', b"must not execute", ""),
            ("opt/local/empty", b'5', b"", ""),
            ("opt/local/bin/app", b'0', b"payload", ""),
            ("opt/local/bin/alias", b'2', b"", "app"),
            ("opt/local/bin/hard", b'1', b"", "opt/local/bin/app"),
        ],
        unsafe { libc::geteuid() }.into(),
        unsafe { libc::getegid() }.into(),
    ));
    extract::prepare(&mut stage(&bytes), &items[0], directory, Limits::default()).unwrap()
}
#[test]
fn maps_payload_and_parent_directories_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    plan.ensure_clear().unwrap();
    assert_eq!(plan.files.len(), 3);
    assert_eq!(plan.directories.len(), 4);
    assert!(
        plan.files
            .iter()
            .any(|f| f.installed_path == "/opt/local/bin/app" && f.member.mode == 0o755)
    );
    assert!(plan.directories.iter().all(|d| !d.exists));
    assert!(
        plan.directories
            .iter()
            .any(|d| d.member_path.as_deref() == Some("opt/local/empty"))
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(
        fs::read(image.payload().join("opt/local/bin/app")).unwrap(),
        b"payload"
    );
}
#[test]
fn shares_directories_but_reports_all_payload_collisions_including_dangling_links() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("opt/local/bin")).unwrap();
    fs::write(root.path().join("opt/local/bin/app"), "existing").unwrap();
    symlink("missing", root.path().join("opt/local/bin/alias")).unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    assert!(plan.ensure_clear().is_err());
    assert_eq!(plan.conflicts.len(), 2);
    assert!(
        plan.conflicts
            .iter()
            .all(|c| c.kind == ConflictKind::ExistingFile)
    );
    assert_eq!(plan.directories.iter().filter(|d| d.exists).count(), 3);
    assert_eq!(
        fs::read_to_string(root.path().join("opt/local/bin/app")).unwrap(),
        "existing"
    );
}
#[test]
fn blocks_symlink_ancestors_without_reading_or_modifying_their_targets() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("opt")).unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].kind, ConflictKind::NonDirectory);
    assert!(plan.directories.is_empty());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
#[test]
fn file_ancestors_and_symlink_roots_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("opt"), "file").unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].kind, ConflictKind::NonDirectory);
    let link = temp.path().join("root-link");
    symlink(root.path(), &link).unwrap();
    assert!(Plan::inspect(&image, &link).is_err());
    assert!(Plan::inspect(&image, &link.join("")).is_err());
}

#[test]
fn registry_preflight_binds_identity_and_detects_owners_without_disk_files() {
    use port_rs::registry::write::{ActivationFile, ImageFile, NewImage, Writer};
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    let mut writer = Writer::create(&temp.path().join("registry.db")).unwrap();
    let mut tx = writer.transaction().unwrap();
    let contents = &image.inspection().contents;
    let mut registered = NewImage {
        name: contents.name.clone(),
        version: contents.version.clone(),
        revision: contents.revision as i64,
        epoch: contents.epoch as i64,
        variants: contents.variants.iter().map(|v| format!("+{v}")).collect(),
        archs: contents.archs.join(" "),
        location: "/images/app.tbz2".into(),
        files: plan
            .files
            .iter()
            .map(|f| ImageFile {
                path: f.installed_path.clone(),
                binary: false,
            })
            .collect(),
        ..NewImage::default()
    };
    let id = tx.register(&registered).unwrap();
    let mappings = plan.check_registry(&tx, id).unwrap();
    assert_eq!(mappings.len(), 3);
    assert!(!tx.entries().unwrap()[0].active);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    registered.version = "2".into();
    let wrong = tx.register(&registered).unwrap();
    assert!(
        plan.check_registry(&tx, wrong)
            .err()
            .unwrap()
            .to_string()
            .contains("identity")
    );
    let owner = tx
        .register(&NewImage {
            name: "owner".into(),
            version: "1".into(),
            location: "/images/owner.tbz2".into(),
            files: vec![ImageFile {
                path: "/owner-file".into(),
                binary: false,
            }],
            ..NewImage::default()
        })
        .unwrap();
    tx.activate(
        owner,
        &[ActivationFile {
            path: "/owner-file".into(),
            actual_path: mappings[0].actual_path.clone(),
        }],
    )
    .unwrap();
    assert!(
        plan.check_registry(&tx, id)
            .err()
            .unwrap()
            .to_string()
            .contains("owned by another image")
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

fn registration(plan: &Plan<'_>) -> port_rs::registry::write::NewImage {
    use port_rs::registry::write::{ImageFile, NewImage};
    let contents = &plan.image.inspection().contents;
    NewImage {
        name: contents.name.clone(),
        version: contents.version.clone(),
        revision: contents.revision as i64,
        epoch: contents.epoch as i64,
        variants: contents.variants.iter().map(|v| format!("+{v}")).collect(),
        archs: contents.archs.join(" "),
        location: "/images/app.tbz2".into(),
        files: plan
            .files
            .iter()
            .map(|f| ImageFile {
                path: f.installed_path.clone(),
                binary: false,
            })
            .collect(),
        ..NewImage::default()
    }
}

#[test]
fn publishes_complete_image_and_commits_registry_before_retention() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    let mut writer =
        port_rs::registry::write::Writer::create(&temp.path().join("registry.db")).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx.register(&registration(&plan)).unwrap();
    let published = plan.publish(&tx, id).unwrap();
    let app = root.path().join("opt/local/bin/app");
    assert_eq!(fs::read(&app).unwrap(), b"payload");
    assert_eq!(
        fs::read_link(root.path().join("opt/local/bin/alias")).unwrap(),
        std::path::Path::new("app")
    );
    let metadata = fs::metadata(&app).unwrap();
    assert_eq!(
        metadata.ino(),
        fs::metadata(root.path().join("opt/local/bin/hard"))
            .unwrap()
            .ino()
    );
    assert_eq!(metadata.mtime(), 1_700_000_000);
    assert_eq!(metadata.mode() & 0o7777, 0o755);
    assert!(root.path().join("opt/local/empty").is_dir());
    assert!(!root.path().join("+PORTFILE").exists());
    assert!(!tx.entries().unwrap()[0].active);
    tx.activate(id, published.mappings()).unwrap();
    tx.commit().unwrap();
    published.persist();
    assert!(writer.transaction().unwrap().entries().unwrap()[0].active);
    assert_eq!(fs::read(&app).unwrap(), b"payload");
}

#[test]
fn rolls_back_the_whole_image_and_preserves_shared_directories() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("opt")).unwrap();
    fs::write(root.path().join("opt/unrelated"), b"retained").unwrap();
    let plan = Plan::inspect(&image, root.path()).unwrap();
    let mut writer =
        port_rs::registry::write::Writer::create(&temp.path().join("registry.db")).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx.register(&registration(&plan)).unwrap();
    let published = plan.publish(&tx, id).unwrap();
    drop(published);
    assert!(!root.path().join("opt/local").exists());
    assert_eq!(
        fs::read(root.path().join("opt/unrelated")).unwrap(),
        b"retained"
    );
    assert!(!tx.entries().unwrap()[0].active);
    plan.publish(&tx, id).unwrap().rollback().unwrap();
    assert!(!root.path().join("opt/local").exists());
    // The old preflight must not authorize overwriting a newly created entry.
    fs::write(root.path().join("opt/local"), b"late collision").unwrap();
    assert!(plan.publish(&tx, id).is_err());
    assert_eq!(
        fs::read(root.path().join("opt/local")).unwrap(),
        b"late collision"
    );
    fs::remove_file(root.path().join("opt/local")).unwrap();
    // A missing staged file fails after directories and the earlier symlink have
    // been created. The entire partial publication must be removed.
    fs::remove_file(image.payload().join("opt/local/bin/app")).unwrap();
    assert!(plan.publish(&tx, id).is_err());
    assert!(!root.path().join("opt/local").exists());
    assert_eq!(
        fs::read(root.path().join("opt/unrelated")).unwrap(),
        b"retained"
    );
}

#[test]
fn coordinated_activation_rolls_back_registry_and_files_on_update_or_commit_failure() {
    for commit_failure in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let image = image(temp.path());
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("opt")).unwrap();
        fs::write(root.path().join("opt/unrelated"), b"retained").unwrap();
        let plan = Plan::inspect(&image, root.path()).unwrap();
        let db_path = temp.path().join("registry.db");
        let mut writer = port_rs::registry::write::Writer::create(&db_path).unwrap();
        let mut tx = writer.transaction().unwrap();
        let id = tx.register(&registration(&plan)).unwrap();
        tx.commit().unwrap();
        let db = rusqlite::Connection::open(&db_path).unwrap();
        if commit_failure {
            // A deferred foreign-key violation occurs only at COMMIT, after
            // every file mapping and the port state have been updated.
            db.execute_batch("CREATE TABLE test_parent (id INTEGER PRIMARY KEY);
                CREATE TABLE test_child (id INTEGER REFERENCES test_parent(id) DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER reject_activation AFTER UPDATE OF state ON ports
                WHEN NEW.state='installed' BEGIN INSERT INTO test_child VALUES (99); END;").unwrap();
        } else {
            db.execute_batch(
                "CREATE TRIGGER reject_activation BEFORE UPDATE OF active ON files
                WHEN NEW.path='/opt/local/bin/hard' AND NEW.active=1
                BEGIN SELECT RAISE(FAIL,'forced activation failure'); END;",
            )
            .unwrap();
        }
        let error = plan.activate(&mut writer, id).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains(if commit_failure {
                "FOREIGN KEY"
            } else {
                "forced activation failure"
            }),
            "{message}"
        );
        assert!(!root.path().join("opt/local").exists());
        assert_eq!(
            fs::read(root.path().join("opt/unrelated")).unwrap(),
            b"retained"
        );
        let tx = writer.transaction().unwrap();
        assert!(!tx.entries().unwrap()[0].active);
        tx.check_activation(id, &plan.check_registry(&tx, id).unwrap())
            .unwrap();
        drop(tx);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM files WHERE active OR actual_path IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER reject_activation").unwrap();
        plan.activate(&mut writer, id).unwrap();
        assert!(writer.transaction().unwrap().entries().unwrap()[0].active);
        assert_eq!(
            fs::read(root.path().join("opt/local/bin/app")).unwrap(),
            b"payload"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM files WHERE active", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}
