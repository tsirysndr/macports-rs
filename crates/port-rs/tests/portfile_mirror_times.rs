use port_rs::{
    install::mirror_times::MirrorTimings,
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};
#[test]
fn policy_cache_age_and_latency_match_upstream_ordering() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(200000);
    let cache = MirrorTimings::new(
        &["*.BLOCKED".into()],
        &["*.preferred".into(), "*.blocked".into()],
    );
    cache
        .record("fast", 1.0, now - Duration::from_secs(86401))
        .unwrap();
    cache.record("slow", 20.0, now).unwrap();
    cache
        .record("boundary", 20.4, now - Duration::from_secs(86400))
        .unwrap();
    assert_eq!(cache.lookup("fast", now), (0, Some(1.0)));
    assert_eq!(cache.lookup("boundary", now), (1, Some(20.4)));
    assert_eq!(cache.lookup("x.blocked", now), (-2, None));
    assert_eq!(cache.lookup("x.preferred", now), (2, None));
    for (a, b, expected) in [
        ("file:///tmp/a", "https://fast/", -1),
        ("file:///tmp/a", "file:///tmp/b", 0),
        ("relative", "https://fast/", 1),
        ("https://fast/", "https://slow/", -19),
        ("https://boundary/", "https://slow/", 0),
        ("https://x.preferred/", "https://slow/", -1),
        ("https://x.blocked/", "https://unknown/", 1),
    ] {
        assert_eq!(cache.compare(a, b, now).unwrap(), expected, "{a} {b}");
    }
    assert!(cache.record("bad", f64::NAN, now).is_err());
}
#[test]
fn runtime_sort_uses_shared_measurements_without_comparator_stub() {
    let cache = MirrorTimings::default();
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    session.set_mirror_timings(cache.clone());
    cache.record("fast", 2.0, SystemTime::now()).unwrap();
    cache.record("slow", 40.0, SystemTime::now()).unwrap();
    session
        .invoke(
            "set",
            &[
                "portfetch::urlmap(master_sites)",
                "https://slow/ https://fast/ file:///tmp/distfiles",
            ],
        )
        .unwrap();
    session
        .invoke("set", &["candidates", "master_sites source.tar.gz"])
        .unwrap();
    session
        .invoke("portfetch::sortsites", &["candidates", "master_sites"])
        .unwrap();
    assert_eq!(
        session
            .invoke("set", &["portfetch::urlmap(master_sites)"])
            .unwrap(),
        "file:///tmp/distfiles https://fast/ https://slow/"
    );
}

#[test]
fn configured_preferences_apply_during_portfile_evaluation() {
    let variables = BTreeMap::from([
        ("preferred_hosts".into(), "*.example".into()),
        ("host_blacklist".into(), "{bad.example} blocked.*".into()),
    ]);
    let mut s = Session::open(
        r#"name app; version 1
        set ranking [compare_pingtimes https://good.example/ https://unknown/]
        set blocked [compare_pingtimes https://bad.example/ https://unknown/]
        # Portfile assignments do not override host policy.
        set preferred_hosts unknown
        "#,
        &variables,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(s.scalar("ranking").unwrap(), "-3");
    assert_eq!(s.scalar("blocked").unwrap(), "1");
    assert_eq!(
        s.invoke(
            "compare_pingtimes",
            &["https://good.example/", "https://unknown/"]
        )
        .unwrap(),
        "-3"
    );
    let bad = BTreeMap::from([("host_blacklist".into(), "{unterminated".into())]);
    assert!(
        Session::open(
            "name app; version 1",
            &bad,
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("invalid host_blacklist list")
    );
}
