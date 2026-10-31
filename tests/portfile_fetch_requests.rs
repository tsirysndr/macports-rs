use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
fn session(path: &std::path::Path) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::from([("distpath".into(), path.to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    s
}
#[test]
fn url_assembly_matches_upstream_character_encoding_without_evaluation() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    for (input, expected) in [
        ("a-Z_0.~", "a-Z_0.~"),
        ("a b/%?", "a%20b%2F%25%3F"),
        ("é€", "%E9%20%AC"),
        ("[x];$y", "%5Bx%5D%3B%24y"),
    ] {
        assert_eq!(s.invoke("percent_encode", &[input]).unwrap(), expected);
        assert_eq!(
            s.invoke("assemble_url", &["https://mirror/base", input])
                .unwrap(),
            format!("https://mirror/base/{expected}")
        );
    }
    assert_eq!(
        s.invoke("assemble_url", &["https://mirror/", "source"])
            .unwrap(),
        "https://mirror/source"
    );
}
#[test]
fn requests_skip_cached_files_sort_once_and_preserve_duplicates() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("cached"), "data").unwrap();
    let mut s = session(root.path());
    s.invoke(
        "set",
        &[
            "portfetch::fetch_urls",
            "master_sites cached missing {source tar.gz} missing {source tar.gz}",
        ],
    )
    .unwrap();
    s.invoke(
        "set",
        &[
            "portfetch::urlmap(master_sites)",
            "https://slow/ file:///local",
        ],
    )
    .unwrap();
    let requests = s.source_fetch_requests().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0].destination, root.path().join("source tar.gz"));
    assert_eq!(
        requests[0].urls,
        [
            "file:///local/source%20tar.gz",
            "https://slow/source%20tar.gz"
        ]
    );
    s.invoke("set", &["sorts", "0"]).unwrap();
    s.invoke(
        "proc",
        &[
            "portfetch::sortsites",
            "urls default",
            "global sorts; incr sorts",
        ],
    )
    .unwrap();
    s.invoke(
        "proc",
        &["assemble_url", "site filename", "return custom:$filename"],
    )
    .unwrap();
    let requests = s.source_fetch_requests().unwrap();
    assert_eq!(s.scalar("sorts").unwrap(), "1");
    assert_eq!(
        requests[0].urls,
        ["custom:source tar.gz", "custom:source tar.gz"]
    );
}
#[test]
fn cached_only_fetch_needs_no_mirrors_and_missing_directory_errors() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("cached"), "data").unwrap();
    let mut s = session(root.path());
    s.invoke("set", &["portfetch::fetch_urls", "unknown cached"])
        .unwrap();
    assert!(s.source_fetch_requests().unwrap().is_empty());
    s.invoke(
        "set",
        &["distpath", root.path().join("missing").to_str().unwrap()],
    )
    .unwrap();
    assert!(
        s.source_fetch_requests()
            .unwrap_err()
            .to_string()
            .contains("must be writable")
    );
}
