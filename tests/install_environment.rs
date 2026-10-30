use port_rs::{
    install::environment::Startup,
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, symlink},
};

fn startup() -> Startup<'static> {
    Startup {
        prefix: "/custom",
        binpath: None,
        extra_env: &[],
        invoking_uid: 501,
        macports_uid: 502,
    }
}

#[test]
fn filters_startup_values_and_overrides_path_and_locale_without_mutating_input() {
    let inherited = BTreeMap::from([
        ("PATH".into(), "/user/bin".into()),
        ("LANG".into(), "C".into()),
        ("LC_ALL".into(), "C".into()),
        ("CC".into(), "custom-cc".into()),
        ("HOME".into(), "/user/home".into()),
        ("http_proxy".into(), "lower".into()),
        ("HTTP_PROXY".into(), "upper".into()),
        ("HTTPS_PROXY".into(), "tls".into()),
        ("DYLD_LIBRARY_PATH".into(), "/libraries".into()),
        ("SSH_AUTH_SOCK".into(), "/agent".into()),
    ]);
    let filtered = startup().prepare(&inherited);
    assert_eq!(
        filtered["PATH"],
        "/custom/bin:/custom/sbin:/bin:/sbin:/usr/bin:/usr/sbin"
    );
    assert_eq!(filtered["LANG"], "en_US.UTF-8");
    for key in ["HOME", "http_proxy", "HTTPS_PROXY", "DYLD_LIBRARY_PATH"] {
        assert_eq!(filtered[key], inherited[key]);
    }
    for key in ["LC_ALL", "CC", "HTTP_PROXY", "SSH_AUTH_SOCK"] {
        assert!(!filtered.contains_key(key));
    }
    assert_eq!(inherited["LANG"], "C");
    assert_eq!(inherited["PATH"], "/user/bin");
    let extra = [
        "CC".into(),
        "HTTP_PROXY".into(),
        "LC_ALL".into(),
        "UNDEFINED".into(),
    ];
    let filtered = Startup {
        binpath: Some(""),
        extra_env: &extra,
        ..startup()
    }
    .prepare(&inherited);
    assert_eq!(filtered["PATH"], "");
    assert_eq!(filtered["CC"], "custom-cc");
    assert_eq!(filtered["LC_ALL"], "C");
    assert_eq!(filtered["HTTP_PROXY"], "upper");
    assert!(!filtered.contains_key("UNDEFINED"));
}

#[test]
fn temporary_paths_follow_writability_owner_and_explicit_extra_rules() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("writable");
    std::fs::write(&path, b"file also accepted upstream").unwrap();
    let link = root.path().join("link");
    symlink(&path, &link).unwrap();
    let owner = path.metadata().unwrap().uid();
    let inherited = BTreeMap::from([
        ("TMP".into(), path.to_str().unwrap().into()),
        ("TMPDIR".into(), link.to_str().unwrap().into()),
    ]);
    assert_eq!(startup().prepare(&inherited)["TMP"], inherited["TMP"]);
    for uid in [0, owner] {
        let result = Startup {
            invoking_uid: 0,
            macports_uid: uid,
            ..startup()
        }
        .prepare(&inherited);
        assert_eq!(result["TMPDIR"], inherited["TMPDIR"]);
    }
    let result = Startup {
        invoking_uid: 0,
        macports_uid: owner.saturating_add(1),
        ..startup()
    }
    .prepare(&inherited);
    assert!(!result.contains_key("TMP"));
    assert!(!result.contains_key("TMPDIR"));
    std::fs::remove_file(&path).unwrap();
    assert!(!startup().prepare(&inherited).contains_key("TMPDIR"));
    let extra = ["TMPDIR".into()];
    assert_eq!(
        Startup {
            extra_env: &extra,
            ..startup()
        }
        .prepare(&inherited)["TMPDIR"],
        inherited["TMPDIR"]
    );
}

#[test]
fn prepared_environment_reaches_portfile_evaluation() {
    let environment = startup().prepare(&BTreeMap::from([("CC".into(), "unwanted".into())]));
    let mut session = Session::open_with_environment(
        "name app; version 1; description $env(PATH); set locale $env(LANG); set compiler_present [info exists env(CC)]",
        &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default(), &environment,
    ).unwrap();
    assert_eq!(
        session.metadata().port.get("description"),
        environment["PATH"]
    );
    assert_eq!(session.scalar("locale").unwrap(), "en_US.UTF-8");
    assert_eq!(session.scalar("compiler_present").unwrap(), "0");
}
