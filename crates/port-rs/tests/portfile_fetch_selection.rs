use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    Session::open(
        r#"
        name app; version 1
        set errors {}
        proc ui_error message {global errors; lappend errors $message}
        set portfetch::urlmap(master_sites) {https://slow/ https://fast/}
        proc compare_pingtimes {a b} {return [string compare $a $b]}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
#[test]
fn sorting_uses_caller_variable_and_falls_back_only_for_missing_tags() {
    let mut s = session();
    s.invoke(
        "proc",
        &[
            "order",
            "",
            r#"
        set candidates {missing one empty two master_sites three}
        set ::portfetch::urlmap(empty) {}
        portfetch::sortsites candidates master_sites
        return $candidates
    "#,
        ],
    )
    .unwrap();
    assert_eq!(
        s.invoke("order", &[]).unwrap(),
        "missing one empty two master_sites three"
    );
    for tag in ["missing", "master_sites"] {
        assert_eq!(
            s.invoke("set", &[&format!("portfetch::urlmap({tag})")])
                .unwrap(),
            "https://fast/ https://slow/"
        );
    }
    assert_eq!(s.invoke("set", &["portfetch::urlmap(empty)"]).unwrap(), "");
    assert!(
        s.scalar("errors")
            .unwrap()
            .contains("No defined site for tag: missing")
    );
    s.invoke("unset", &["portfetch::urlmap(master_sites)"])
        .unwrap();
    s.invoke("set", &["candidates", "master_sites file"])
        .unwrap();
    s.invoke("portfetch::sortsites", &["candidates", "master_sites"])
        .unwrap();
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(master_sites)"])
            .unwrap(),
        ""
    );
}
#[test]
fn url_collection_initializes_files_and_preserves_duplicates_and_order() {
    let mut s = session();
    s.invoke(
        "proc",
        &[
            "portfetch::checkfiles",
            "urls",
            r#"
        upvar $urls candidates
        set candidates {missing first master_sites second}
    "#,
        ],
    )
    .unwrap();
    assert_eq!(
        s.invoke("portfetch::get_urls", &[]).unwrap(),
        "https://slow/ https://fast/ https://slow/ https://fast/"
    );
    s.invoke("proc", &["compare_pingtimes", "a b", "error ping-failed"])
        .unwrap();
    assert!(
        s.invoke(
            "portfetch::sortsites",
            &["portfetch::fetch_urls", "master_sites"]
        )
        .unwrap_err()
        .to_string()
        .contains("ping-failed")
    );
    s.invoke(
        "unset",
        &[
            "portfetch::urlmap(missing)",
            "portfetch::urlmap(master_sites)",
        ],
    )
    .unwrap();
    assert!(s.invoke("portfetch::get_urls", &[]).is_err());
}
