use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
};
fn session(execution: bool) -> Session {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        session
            .enable_system(&BTreeMap::new(), |_| {
                panic!("ownership requires no subprocess")
            })
            .unwrap();
    }
    session
}
#[test]
fn attributes_round_trip_native_names_and_numeric_ids_without_changing_other_identity() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "data").unwrap();
    let path = file.to_str().unwrap();
    let original = fs::metadata(&file).unwrap();
    let mut query = session(false);
    let owner = query
        .invoke("file", &["attributes", path, "-owner"])
        .unwrap();
    let group = query
        .invoke("file", &["attributes", path, "-group"])
        .unwrap();
    assert!(!owner.is_empty());
    assert!(!group.is_empty());
    let mut execution = session(true);
    for (attribute, value) in [
        ("-owner", owner),
        ("-group", group),
        ("-o", original.uid().to_string()),
        ("-g", original.gid().to_string()),
        ("-owner", "-1".into()),
        ("-group", "-1".into()),
    ] {
        execution
            .invoke("file", &["attributes", path, attribute, &value])
            .unwrap();
        let after = fs::metadata(&file).unwrap();
        assert_eq!(after.uid(), original.uid());
        assert_eq!(after.gid(), original.gid());
    }
    execution
        .invoke(
            "file",
            &[
                "attributes",
                path,
                "-owner",
                &original.uid().to_string(),
                "-group",
                &original.gid().to_string(),
                "-permissions",
                "0640",
            ],
        )
        .unwrap();
    assert_eq!(fs::metadata(file).unwrap().mode() & 0o777, 0o640);
}
#[test]
fn unknown_accounts_and_missing_files_produce_catchable_native_error_codes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "data").unwrap();
    let mut session = session(true);
    session
        .invoke("set", &["path", file.to_str().unwrap()])
        .unwrap();
    for (attribute, code) in [
        ("-owner", "TCL OPERATION SETOWN NO_USER"),
        ("-group", "TCL OPERATION SETGRP NO_GROUP"),
    ] {
        session.invoke("set", &["attribute", attribute]).unwrap();
        assert_eq!(
            session
                .invoke(
                    "catch",
                    &[
                        "file attributes $path $attribute port-rs-account-that-does-not-exist",
                        "message"
                    ]
                )
                .unwrap(),
            "1"
        );
        assert_eq!(session.scalar("errorCode").unwrap(), code);
        assert!(
            session
                .scalar("message")
                .unwrap()
                .contains("does not exist")
        );
    }
    session
        .invoke(
            "set",
            &["path", root.path().join("missing").to_str().unwrap()],
        )
        .unwrap();
    session
        .invoke("catch", &["file attributes $path -owner -1"])
        .unwrap();
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX ENOENT {no such file or directory}"
    );
}
#[test]
fn owner_group_queries_follow_links_and_are_safe_to_run_concurrently() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "data").unwrap();
    let link = root.path().join("link");
    symlink(&file, &link).unwrap();
    let mut session = session(false);
    let name = session
        .invoke("file", &["attributes", file.to_str().unwrap(), "-owner"])
        .unwrap();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let path = link.clone();
            std::thread::spawn(move || {
                let mut session = crate::session(false);
                (0..20)
                    .map(|_| {
                        session
                            .invoke("file", &["attributes", path.to_str().unwrap(), "-owner"])
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for worker in workers {
        assert!(worker.join().unwrap().iter().all(|found| found == &name));
    }
}
#[test]
fn metadata_cannot_set_ownership_even_with_noop_ids() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "data").unwrap();
    for attribute in ["-owner", "-group"] {
        assert!(
            Session::open(
                &format!("name app; version 1; catch {{file attributes $path {attribute} -1}}"),
                &BTreeMap::from([("path".into(), file.to_str().unwrap().into())]),
                &PortSpec::named("app"),
                PortGroups::default()
            )
            .is_err()
        );
    }
}
