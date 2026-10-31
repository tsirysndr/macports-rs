use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session, TargetOutcome},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
#[test]
fn candidates_preserve_caller_locals_tags_and_patch_before_dist_order() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("local.patch"), b"local").unwrap();
    let mut s = Session::open(
        r#"
        name app; version 1; license NoMirror
        patch_sites https://patch/
        master_sites https://dist/
        patchfiles local.patch remote.patch other.patch:special
        distfiles source.tar.xz:release plain.tar.gz
        proc portfetch::get_full_mirror_sites_path {} {return /tree/mirrors.tcl}
        proc collect {} {
            set urls {}
            portfetch::checkfiles urls
            return $urls
        }
    "#,
        &BTreeMap::from([("filespath".into(), tmp.path().to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let expected = "patch_sites remote.patch special other.patch release source.tar.xz master_sites plain.tar.gz";
    assert_eq!(s.invoke("collect", &[]).unwrap(), expected);
    assert_eq!(
        s.scalar("all_dist_files").unwrap(),
        "remote.patch other.patch source.tar.xz plain.tar.gz"
    );
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(master_sites)"])
            .unwrap(),
        "https://dist/"
    );
    assert_eq!(s.invoke("getdistname", &["a:tag-1"]).unwrap(), "a");
    assert_eq!(s.invoke("getdisttag", &["a:tag-1"]).unwrap(), "tag-1");
    assert_eq!(
        s.invoke("getdistname", &["a:bad/tag"]).unwrap(),
        "a:bad/tag"
    );
    assert_eq!(s.invoke("getdisttag", &[":tag"]).unwrap(), "");
    s.invoke("unset", &["all_dist_files"]).unwrap();
    s.invoke("portfetch::fetch_init", &["ignored"]).unwrap();
    assert_eq!(s.scalar("portfetch::fetch_urls").unwrap(), expected);
    let before = s.scalar("all_dist_files").unwrap();
    s.invoke("portfetch::fetch_init", &[]).unwrap();
    assert_eq!(s.scalar("all_dist_files").unwrap(), before);
    // A completed fetch still initializes, then skips its source body/hooks.
    s.enable_system(&BTreeMap::new(), |_| panic!("no process expected"))
        .unwrap();
    let mut state = StateFile::open(&tmp.path().join("state"), None).unwrap();
    state.record("target", "org.macports.fetch").unwrap();
    let fd = s.attach_build_state(state).unwrap();
    let target = s.scalar("org.macports.fetch").unwrap();
    assert_eq!(
        s.execute_target(&target, Some(&fd), false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
}
#[test]
fn empty_patch_sites_still_selects_patch_tag_and_missing_list_uses_master() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = Session::open(
        "name app; version 1; patchfiles remote.patch",
        &BTreeMap::from([("filespath".into(), tmp.path().to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.invoke("portfetch::checkpatchfiles", &["urls"]).unwrap();
    assert_eq!(s.scalar("urls").unwrap(), "master_sites remote.patch");
    s.invoke("set", &["urls", ""]).unwrap();
    s.invoke("patch_sites", &[]).unwrap();
    s.invoke("portfetch::checkpatchfiles", &["urls"]).unwrap();
    assert_eq!(s.scalar("urls").unwrap(), "patch_sites remote.patch");
}
