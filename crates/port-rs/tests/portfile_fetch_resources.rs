use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
#[test]
fn selected_tree_resource_drives_native_fetch_initialization_and_is_cached() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("_resources/port1.0/fetch");
    fs::create_dir_all(&dir).unwrap();
    let resource = dir.join("mirror_sites.tcl");
    fs::write(&resource, r#"namespace eval ::portfetch::mirror_sites {
        namespace upvar :: os.major major
        variable sites
        if {$major >= 24} {set sites(example) {https://new/:nosubdir}} else {set sites(example) {https://old/:nosubdir}}
        set sites(macports_distfiles) {https://global/:mirror}
    }"#).unwrap();
    let mut s = Session::open(
        r#"
        name app; version 1; license MIT
        set os.platform darwin; set os.major 24
        master_sites example
        distfiles app.tar.xz
    "#,
        &BTreeMap::from([(
            "filespath".into(),
            root.path().join("files").to_str().unwrap().into(),
        )]),
        &PortSpec::named("app"),
        PortGroups::from_tree(root.path()),
    )
    .unwrap();
    let path = s
        .invoke("portfetch::get_full_mirror_sites_path", &[])
        .unwrap();
    assert_eq!(path, resource.to_str().unwrap());
    s.invoke("portfetch::fetch_init", &[]).unwrap();
    assert_eq!(
        s.scalar("portfetch::fetch_urls").unwrap(),
        "master_sites app.tar.xz"
    );
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(master_sites)"])
            .unwrap(),
        "https://new/ https://global/app"
    );
    fs::write(&resource, "error changed-after-first-read").unwrap();
    assert_eq!(
        s.invoke("portfetch::get_mirror_site_urls", &[&path, "example"])
            .unwrap(),
        "https://new/:nosubdir"
    );
    assert_eq!(
        s.invoke("portfetch::get_mirror_site_urls", &[&path, "unknown"])
            .unwrap(),
        ""
    );
    let other = dir.join("other.tcl");
    fs::write(&other, "exec forbidden").unwrap();
    assert!(
        s.invoke(
            "portfetch::get_mirror_site_urls",
            &[other.to_str().unwrap(), "example"]
        )
        .is_err()
    );
    let missing = dir.join("missing.tcl");
    assert_eq!(
        s.invoke(
            "portfetch::get_mirror_site_urls",
            &[missing.to_str().unwrap(), "example"]
        )
        .unwrap(),
        ""
    );
    assert!(!missing.exists());
}
