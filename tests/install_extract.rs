#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::{
    index::Index,
    install::{self, extract, inspect::Limits},
    spec::PortSpec,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn index() -> Index {
    let body = "name app version 1 revision 0 variants ssl vinfo {ssl {is_default +}}";
    Index::parse(&format!("app {}\n{body}\n", body.len() + 1)).unwrap()
}
#[test]
fn prepares_files_controls_directories_and_links_without_publishing() {
    let c = contents(&["opt/local/app", "opt/local/hard", "opt/local/link"]);
    let data = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("+PORTFILE", b'0', b"exec must-not-run", ""),
        ("opt/local/empty", b'5', b"", ""),
        ("opt/local/app", b'0', b"hello", ""),
        ("opt/local/hard", b'1', b"", "opt/local/app"),
        ("opt/local/link", b'2', b"", "app"),
    ]));
    let mut archive = stage(&data);
    let directory = tempfile::tempdir().unwrap();
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let image =
        extract::prepare(&mut archive, &plan[0], directory.path(), Limits::default()).unwrap();
    let root = image.payload();
    assert_eq!(std::fs::read(root.join("opt/local/app")).unwrap(), b"hello");
    assert_eq!(
        std::fs::read(image.controls().join("+PORTFILE")).unwrap(),
        b"exec must-not-run"
    );
    assert!(!root.join("+CONTENTS").exists());
    assert!(root.join("opt/local/empty").is_dir());
    assert_eq!(
        std::fs::metadata(root.join("opt/local/app")).unwrap().ino(),
        std::fs::metadata(root.join("opt/local/hard"))
            .unwrap()
            .ino()
    );
    assert_eq!(
        std::fs::read_link(root.join("opt/local/link"))
            .unwrap()
            .to_str(),
        Some("app")
    );
    assert_eq!(
        std::fs::metadata(root.join("opt/local/app"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(image.inspection().members["opt/local/app"].mode, 0o755);
    drop(image);
    assert!(!root.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
#[test]
fn external_symlink_targets_are_not_followed_or_modified() {
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("untouched");
    std::fs::write(&outside, b"keep").unwrap();
    let c = contents(&["opt/local/link"]);
    let bytes = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/link", b'2', b"", outside.to_str().unwrap()),
    ]));
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let image = extract::prepare(
        &mut stage(&bytes),
        &plan[0],
        directory.path(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_link(image.payload().join("opt/local/link")).unwrap(),
        outside
    );
    drop(image);
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}
#[test]
fn identity_and_path_validation_fail_before_creating_an_image() {
    let directory = tempfile::tempdir().unwrap();
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let wrong = contents(&[]).replace("app", "other");
    let c = contents(&["opt/link/file", "opt/link"]);
    for raw in [
        tar(&[("+CONTENTS", b'0', wrong.as_bytes(), "")]),
        tar(&[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("opt/link/file", b'0', b"x", ""),
            ("opt/link", b'2', b"", "/tmp"),
        ]),
    ] {
        assert!(
            extract::prepare(
                &mut stage(&compress(&raw)),
                &plan[0],
                directory.path(),
                Limits::default()
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
#[test]
fn stages_payload_in_archive_order_even_when_path_sort_order_differs() {
    let c = contents(&["z/file", "a/file", "empty"]);
    let bytes = compress(&tar(&[
        ("z/file", b'0', b"last by name", ""),
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("empty", b'0', b"", ""),
        ("a/file", b'0', b"first by name", ""),
    ]));
    let directory = tempfile::tempdir().unwrap();
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let image = extract::prepare(
        &mut stage(&bytes),
        &plan[0],
        directory.path(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(image.payload().join("z/file")).unwrap(),
        b"last by name"
    );
    assert_eq!(
        std::fs::read(image.payload().join("a/file")).unwrap(),
        b"first by name"
    );
    assert_eq!(
        std::fs::metadata(image.payload().join("empty"))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn filesystem_failure_cleans_up_partially_prepared_image() {
    let long = "x".repeat(300);
    let body = format!(" path={long}\n");
    let mut length = body.len() + 1;
    loop {
        let next = body.len() + length.to_string().len();
        if next == length {
            break;
        }
        length = next;
    }
    let pax = format!("{length}{body}");
    let c = contents(&["first", &long]);
    let bytes = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("first", b'0', b"written first", ""),
        ("PaxHeader", b'x', pax.as_bytes(), ""),
        ("placeholder", b'0', b"cannot create", ""),
    ]));
    let directory = tempfile::tempdir().unwrap();
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let result = extract::prepare(
        &mut stage(&bytes),
        &plan[0],
        directory.path(),
        Limits::default(),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn target_architectures_are_checked_before_extraction() {
    use port_rs::{
        install::target::{Architecture, Target},
        source::Platform,
    };
    let c = contents(&["opt/local/app"]);
    let data = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/app", b'0', b"hello", ""),
    ]));
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let platform = Platform {
        darwin_major: 24,
        architecture: "arm",
    };
    for architecture in [
        Architecture::Build("x86_64"),
        Architecture::Noarch,
        Architecture::Universal(&["arm64", "x86_64"]),
    ] {
        let target = Target::macos(&platform, "darwin", architecture).unwrap();
        let result = extract::prepare_for_target(
            &mut stage(&data),
            &plan[0],
            &target,
            directory.path(),
            Limits::default(),
        );
        assert!(result.err().unwrap().to_string().contains("architectures"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    let target = Target::macos(&platform, "darwin", Architecture::Native).unwrap();
    let image = extract::prepare_for_target(
        &mut stage(&data),
        &plan[0],
        &target,
        directory.path(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(image.payload().join("opt/local/app")).unwrap(),
        b"hello"
    );
}

#[test]
fn builds_registry_inventory_with_binary_flags_without_following_symlinks() {
    let c = contents(&[
        "opt/local/app",
        "opt/local/hard",
        "opt/local/link",
        "opt/local/text",
        "opt/local/static.a",
    ]);
    let binary = 0xfeedfacfu32.to_ne_bytes();
    let data = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("+PORTFILE", b'0', &binary, ""),
        ("opt/local/empty", b'5', b"", ""),
        ("opt/local/app", b'0', &binary, ""),
        ("opt/local/hard", b'1', b"", "opt/local/app"),
        ("opt/local/link", b'2', b"", "/nonexistent/outside"),
        ("opt/local/text", b'0', b"hello", ""),
        ("opt/local/static.a", b'0', b"!<arch>\n", ""),
    ]));
    let directory = tempfile::tempdir().unwrap();
    let index = index();
    let plan = install::plan(&index, &[PortSpec::named("app")], &[]).unwrap();
    let image = extract::prepare(
        &mut stage(&data),
        &plan[0],
        directory.path(),
        Limits::default(),
    )
    .unwrap();
    let files: std::collections::BTreeMap<_, _> = image
        .registry_files()
        .unwrap()
        .into_iter()
        .map(|f| (f.path, f.binary))
        .collect();
    assert_eq!(
        files,
        std::collections::BTreeMap::from([
            ("/opt/local/app".into(), true),
            ("/opt/local/hard".into(), true),
            ("/opt/local/link".into(), false),
            ("/opt/local/text".into(), false),
            ("/opt/local/static.a".into(), true),
        ])
    );
    std::fs::remove_file(image.payload().join("opt/local/text")).unwrap();
    std::os::unix::fs::symlink("app", image.payload().join("opt/local/text")).unwrap();
    assert!(image.registry_files().is_err());
}
