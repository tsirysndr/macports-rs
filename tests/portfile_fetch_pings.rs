use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    let mut s = Session::open(r#"
        name app; version 1; license MIT; set events {}
        set portfetch::fetch_urls {one a missing b two c}
        set portarchivefetch::archivefetch_urls {two archive}
        set portfetch::urlmap(one) {https://one/}
        set portfetch::urlmap(two) {https://two/}
        proc async_ping_start urls {global events; lappend events $urls}
        proc wait_for_pingtime url {global events; lappend events wait $url}
        proc portfetch::get_full_mirror_sites_path {} {return {resource with spaces}}
        proc portfetch::get_mirror_site_urls {path class} {global events; lappend events $path $class; return {https://primary/ https://secondary/}}
        proc portarchivefetch::get_full_archive_sites_path {} {return archive-resource}
        proc portarchivefetch::get_archive_site_urls {path class} {global events; lappend events $path $class; return {https://archive/}}
    "#, &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    s
}
#[test]
fn source_probes_candidates_before_optional_primary_wait() {
    let mut s = session();
    s.invoke("portfetch::start_pings", &[]).unwrap();
    assert_eq!(
        s.scalar("events").unwrap(),
        "https://one/ https://two/ {resource with spaces} macports_distfiles wait https://primary/"
    );
    for flag in ["yes", "false", "0"] {
        s.invoke("set", &["ports_fetch_no-mirrors", flag]).unwrap();
        s.invoke("set", &["events", ""]).unwrap();
        s.invoke("portfetch::start_pings", &[]).unwrap();
        assert_eq!(s.scalar("events").unwrap(), "https://one/ https://two/");
    }
    s.invoke("set", &["ports_fetch_no-mirrors", "no"]).unwrap();
    s.invoke("license", &["NoMirror"]).unwrap();
    s.invoke("set", &["events", ""]).unwrap();
    s.invoke("portfetch::start_pings", &[]).unwrap();
    assert_eq!(s.scalar("events").unwrap(), "https://one/ https://two/");
}
#[test]
fn archive_prefers_official_class_and_propagates_probe_errors() {
    for (sites, expected) in [
        ("custom::tbz2 macports_archives::tbz2", "macports_archives"),
        ("custom::tbz2 other::tbz2", "custom"),
    ] {
        let mut s = session();
        s.invoke("set", &["archive_sites", sites]).unwrap();
        s.invoke("portarchivefetch::start_pings", &[]).unwrap();
        assert_eq!(
            s.scalar("events").unwrap(),
            format!("https://two/ archive-resource {expected} wait https://archive/")
        );
    }
    let mut s = session();
    s.invoke("proc", &["async_ping_start", "urls", "error probe-failed"])
        .unwrap();
    assert!(
        s.invoke("portarchivefetch::start_pings", &[])
            .unwrap_err()
            .to_string()
            .contains("probe-failed")
    );
    assert_eq!(s.scalar("events").unwrap(), "");
}

#[test]
fn native_source_initialization_probes_resource_mirrors_with_real_worker() {
    use port_rs::install::{mirror_times::MirrorTimings, mirror_worker::MirrorWorker};
    let root = tempfile::tempdir().unwrap();
    let resource = root.path().join("_resources/port1.0/fetch");
    std::fs::create_dir_all(&resource).unwrap();
    std::fs::write(
        resource.join("mirror_sites.tcl"),
        r#"
        namespace eval ::portfetch::mirror_sites {
            variable sites
            set sites(macports_distfiles) {nosuchservice://127.0.0.1/:mirror}
        }
    "#,
    )
    .unwrap();
    let mut s = Session::open(
        r#"
        name app; version 1; license MIT
        set os.platform darwin; set os.major 24
        master_sites {}; distfiles app.tar.gz
    "#,
        &BTreeMap::from([(
            "filespath".into(),
            root.path().join("files").to_str().unwrap().into(),
        )]),
        &PortSpec::named("app"),
        PortGroups::from_tree(root.path()),
    )
    .unwrap();
    let cache = MirrorTimings::default();
    s.set_mirror_worker(MirrorWorker::new(cache.clone(), 2).unwrap());
    s.enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    s.invoke("portfetch::fetch_init", &[]).unwrap();
    s.invoke("portfetch::start_pings", &[]).unwrap();
    assert_eq!(
        cache.lookup("127.0.0.1", std::time::SystemTime::now()),
        (1, Some(10000.0))
    );
}
