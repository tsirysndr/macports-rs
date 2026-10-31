use port_rs::{
    portfile::{PortGroups, Session},
    progress::Progress,
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    let mut s = Session::open(r#"
        name app; version 1; set events {}; set completed 0; set cached 0; set candidates 1
        proc portarchivefetch::check_destroot_done {} {global completed events; lappend events state; return $completed}
        proc portarchivefetch::checkfiles urls {
            global events candidates all_archive_files
            upvar $urls files
            lappend events urls
            if {$candidates} {set all_archive_files {app.tbz2}; set files {tbz2 app.tbz2}}
        }
        proc portarchivefetch::archives_present {} {global events cached; lappend events cache; return $cached}
        proc portarchivefetch::start_pings {} {global events; lappend events ping}
        proc portfetch::check_dns {} {global events; lappend events dns}
        proc portarchivefetch::fetchfiles {} {global events; lappend events fetch}
    "#, &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    s.set_progress(Progress::new(true));
    s
}
#[test]
fn archive_start_and_main_follow_source_binary_and_cache_policy() {
    for (flag, cached, expected) in [
        ("normal", "0", "state urls cache ping dns fetch"),
        ("normal", "1", "state urls cache dns fetch"),
        ("ports_source_only", "0", "dns"),
        ("completed", "0", "state dns"),
        ("ports_binary_only", "0", "urls cache ping dns fetch"),
    ] {
        let mut s = session();
        s.invoke("set", &[flag, "1"]).unwrap();
        s.invoke("set", &["cached", cached]).unwrap();
        s.invoke("portarchivefetch::_archivefetch_start", &["yes"])
            .unwrap();
        assert_eq!(
            s.invoke("portarchivefetch::archivefetch_main", &[])
                .unwrap(),
            "0"
        );
        assert_eq!(s.scalar("events").unwrap(), expected);
        if expected.contains("urls") {
            assert_eq!(
                s.scalar("portarchivefetch::archivefetch_urls").unwrap(),
                "tbz2 app.tbz2"
            );
        }
    }
}
#[test]
fn unusable_binary_sites_and_callback_errors_stop_startup() {
    let mut s = session();
    s.invoke("set", &["ports_binary_only", "yes"]).unwrap();
    s.invoke("set", &["candidates", "0"]).unwrap();
    assert!(
        s.invoke("portarchivefetch::archivefetch_start", &[])
            .unwrap_err()
            .to_string()
            .contains("no usable archive sites")
    );
    assert_eq!(s.scalar("events").unwrap(), "urls");
    let mut s = session();
    s.invoke(
        "proc",
        &["portarchivefetch::start_pings", "", "error ping-failed"],
    )
    .unwrap();
    assert!(
        s.invoke("portarchivefetch::archivefetch_start", &[])
            .unwrap_err()
            .to_string()
            .contains("ping-failed")
    );
    assert_eq!(s.scalar("events").unwrap(), "state urls cache");
}
