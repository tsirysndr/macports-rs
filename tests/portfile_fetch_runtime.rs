use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
fn session() -> Session {
    Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
#[test]
fn cached_candidates_require_regular_files_and_follow_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session();
    // Empty lists need no distpath, matching the upstream foreach.
    assert_eq!(s.invoke("portfetch::files_present", &[]).unwrap(), "1");
    s.invoke("set", &["distpath", root.path().to_str().unwrap()])
        .unwrap();
    s.invoke(
        "set",
        &["portfetch::fetch_urls", "master_sites one patch_sites two"],
    )
    .unwrap();
    fs::write(root.path().join("one"), "data").unwrap();
    fs::create_dir(root.path().join("two")).unwrap();
    assert_eq!(s.invoke("portfetch::files_present", &[]).unwrap(), "0");
    fs::remove_dir(root.path().join("two")).unwrap();
    std::os::unix::fs::symlink("one", root.path().join("two")).unwrap();
    assert_eq!(s.invoke("portfetch::files_present", &[]).unwrap(), "1");
    fs::remove_file(root.path().join("one")).unwrap();
    assert_eq!(s.invoke("portfetch::files_present", &[]).unwrap(), "0");
}
#[test]
fn dispatch_preserves_return_values_and_errors_and_skips_empty_standard_fetch() {
    let mut s = session();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    assert_eq!(
        s.invoke("portfetch::fetch_main", &["ignored"]).unwrap(),
        "0"
    );
    for kind in ["bzr", "cvs", "svn", "git", "hg", "standard", "unknown"] {
        let command = match kind {
            "standard" | "unknown" => "portfetch::fetchfiles".to_owned(),
            _ => format!("portfetch::{kind}fetch"),
        };
        s.invoke("proc", &[&command, "", "return fetched"]).unwrap();
        s.invoke("fetch.type", &[kind]).unwrap();
        // Standard fetch tests existence, including an explicitly empty list.
        s.invoke("set", &["all_dist_files", ""]).unwrap();
        assert_eq!(s.invoke("portfetch::fetch_main", &[]).unwrap(), "fetched");
        s.invoke("proc", &[&command, "", "error transport-failed"])
            .unwrap();
        assert!(
            s.invoke("portfetch::fetch_main", &[])
                .unwrap_err()
                .to_string()
                .contains("transport-failed")
        );
    }
    let mut unbound = session();
    assert!(unbound.invoke("portfetch::fetch_main", &[]).is_err());
}
