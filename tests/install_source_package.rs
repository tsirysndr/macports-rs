use port_rs::install::{
    archive::VerifiedArchive,
    inspect::{self, Limits},
    source::package::{self, Inputs, Platform},
};
use std::{fs, os::unix::fs::symlink};

#[test]
fn packaged_destroot_round_trips_through_archive_inspection() {
    let root = tempfile::tempdir().unwrap();
    let destroot = root.path().join("destroot");
    fs::create_dir_all(destroot.join("opt/local/bin")).unwrap();
    fs::create_dir_all(destroot.join("opt/local/share/doc/app")).unwrap();
    fs::write(destroot.join("opt/local/share/doc/app/README"), "readme").unwrap();
    fs::copy("/usr/bin/true", destroot.join("opt/local/bin/tool")).unwrap();
    symlink("tool", destroot.join("opt/local/bin/alias")).unwrap();
    fs::write(root.path().join("state"), "version: 2\n").unwrap();
    fs::write(root.path().join("Portfile"), "name app\n").unwrap();
    let inputs = Inputs {
        destroot: destroot.clone(),
        statefile: root.path().join("state"),
        portfile: root.path().join("Portfile"),
        subport: "app".into(),
        version: "1.2".into(),
        revision: 3,
        epoch: 0,
        variants: vec!["x11".into(), "docs".into()],
        archs: "arm64".into(),
        dependencies: vec!["zlib-1.3.1_0".into()],
        description: Some("An app".into()),
        long_description: Some("A longer description".into()),
        homepage: Some("https://example.org".into()),
        platform: Some(Platform {
            subplatform: "macosx".into(),
            os_version: Some("24.6.0".into()),
            macos_version: Some("15.6".into()),
            toolchain: Some(("xcodecltversion".into(), "16.4".into())),
        }),
        ports_commit: None,
        source_date_epoch: "1700000000".into(),
        macports_version: "2.12.7".into(),
        configured_cxx_stdlib: "libc++".into(),
        default_cxx_stdlib: "libc++".into(),
    };
    let output = root
        .path()
        .join("software/app/app-1.2_3+docs+x11.darwin_24.arm64.tbz2");
    package::create(&inputs, &output).unwrap();
    assert!(
        fs::read_dir(&destroot).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('+')),
        "control files are removed from the destroot"
    );
    let mut archive = VerifiedArchive::local(fs::File::open(&output).unwrap()).unwrap();
    assert!(archive.signature().is_empty());
    let inspection = inspect::tbz2(&mut archive, Limits::default()).unwrap();
    let contents = &inspection.contents;
    assert_eq!(contents.name, "app");
    assert_eq!(contents.version, "1.2");
    assert_eq!(contents.revision, 3);
    assert_eq!(
        contents.variants.iter().cloned().collect::<Vec<_>>(),
        ["docs", "x11"]
    );
    assert_eq!(contents.archs, ["arm64"]);
    assert_eq!(contents.dependencies, ["zlib-1.3.1_0"]);
    assert_eq!(contents.cxx_stdlib.as_deref(), Some("none"));
    assert_eq!(
        contents.files.iter().cloned().collect::<Vec<_>>(),
        [
            "opt/local/bin/alias",
            "opt/local/bin/tool",
            "opt/local/share/doc/app/README"
        ]
    );
    for control in ["+CONTENTS", "+PORTFILE", "+STATE", "+COMMENT", "+DESC"] {
        assert!(inspection.members.contains_key(control), "{control}");
    }
}

#[test]
fn packaging_requires_a_destroot_and_cleans_up_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let inputs = Inputs {
        destroot: root.path().join("missing"),
        ..Inputs::default()
    };
    let output = root.path().join("out.tbz2");
    assert!(
        package::create(&inputs, &output)
            .unwrap_err()
            .to_string()
            .contains("no destroot")
    );
    fs::create_dir(root.path().join("destroot")).unwrap();
    let inputs = Inputs {
        destroot: root.path().join("destroot"),
        statefile: root.path().join("absent-state"),
        ..Inputs::default()
    };
    assert!(package::create(&inputs, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn cxx_stdlib_detection_reads_linked_runtimes() {
    assert_eq!(package::actual_cxx_stdlib(&[]), "none");
    assert_eq!(
        package::actual_cxx_stdlib(&["/usr/bin/true".into()]),
        "none"
    );
}

#[test]
fn long_paths_and_link_targets_use_pax_records() {
    let root = tempfile::tempdir().unwrap();
    let destroot = root.path().join("destroot");
    let deep = format!(
        "opt/{}/{}/{}",
        "a".repeat(90),
        "b".repeat(90),
        "c".repeat(90)
    );
    fs::create_dir_all(destroot.join(&deep)).unwrap();
    fs::write(destroot.join(&deep).join("file.txt"), "deep").unwrap();
    let target = format!("{}/file.txt", "x".repeat(120));
    symlink(&target, destroot.join("opt/link")).unwrap();
    fs::write(root.path().join("state"), "").unwrap();
    fs::write(root.path().join("Portfile"), "").unwrap();
    let inputs = Inputs {
        destroot,
        statefile: root.path().join("state"),
        portfile: root.path().join("Portfile"),
        subport: "deep".into(),
        version: "1".into(),
        archs: "noarch".into(),
        source_date_epoch: "0".into(),
        macports_version: "2.12.7".into(),
        ..Inputs::default()
    };
    let output = root.path().join("deep.tbz2");
    package::create(&inputs, &output).unwrap();
    let mut archive = VerifiedArchive::local(fs::File::open(&output).unwrap()).unwrap();
    let inspection = inspect::tbz2(&mut archive, Limits::default()).unwrap();
    let files: Vec<_> = inspection.contents.files.iter().cloned().collect();
    assert_eq!(files, [format!("{deep}/file.txt"), "opt/link".to_string()]);
    assert!(inspection.members.contains_key(&format!("{deep}/file.txt")));
}
