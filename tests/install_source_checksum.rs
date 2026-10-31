use port_rs::{
    install::source::checksum::{self, Kind, Problem},
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};

const MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
const SHA1: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";
const RMD160: &str = "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc";
const SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(Into::into).collect()
}

#[test]
fn computes_all_upstream_types_in_one_pass() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("abc");
    fs::write(&path, "abc").unwrap();
    let kinds = [
        Kind::Sha256,
        Kind::Size,
        Kind::Md5,
        Kind::Rmd160,
        Kind::Sha1,
    ];
    assert_eq!(
        checksum::calculate(&path, &kinds).unwrap(),
        [SHA256, "3", MD5, RMD160, SHA1]
    );
}

#[test]
fn parses_like_upstream_including_lenient_and_repeated_entries() {
    let one = ["a.tgz".to_string()];
    let table = checksum::parse(&words("rmd160 x size 3"), &one);
    assert_eq!(
        table["a.tgz"],
        [(Kind::Rmd160, "x".into()), (Kind::Size, "3".into())]
    );
    let table = checksum::parse(&words("a.tgz sha256 1"), &one);
    assert_eq!(table["a.tgz"], [(Kind::Sha256, "1".into())]);
    let two = ["a.tgz".to_string(), "b.tgz".to_string()];
    let table = checksum::parse(
        &words("a.tgz sha256 1 b.tgz md5 2 size 4 a.tgz size 9"),
        &two,
    );
    assert_eq!(
        table["a.tgz"],
        [(Kind::Sha256, "1".into()), (Kind::Size, "9".into())]
    );
    assert_eq!(table["b.tgz"].len(), 2);
    let table = checksum::parse(&words("a.tgz b.tgz size"), &two);
    assert!(table["a.tgz"].is_empty());
    assert_eq!(table["b.tgz"], [(Kind::Size, String::new())]);
    let table = checksum::parse(&words("rmd160 x sha1 y md5 z size 1 sha256 2 size 3"), &one);
    assert!(!table.contains_key("a.tgz"));
}

#[test]
fn verify_reports_every_mismatch_and_missing_entry() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), "abc").unwrap();
    fs::write(root.path().join("b"), "abc").unwrap();
    let files = ["a".to_string(), "b".to_string()];
    let table = checksum::parse(&words(&format!("a sha256 {SHA256} size 4 b")), &files);
    assert_eq!(
        checksum::verify(root.path(), &files, &table).unwrap(),
        [
            Problem::Mismatch {
                distfile: "a".into(),
                kind: Kind::Size,
                expected: "4".into(),
                actual: "3".into(),
            },
            Problem::Missing("b".into()),
        ]
    );
    let pad = " ".repeat(20);
    assert_eq!(
        checksum::suggestion(root.path(), &files, &table).unwrap(),
        format!(
            "checksums           a \\\n{pad}sha256  {SHA256} \\\n{pad}size    3 \\\n{pad}rmd160  {RMD160} \\\n{pad}b \\\n{pad}rmd160  {RMD160} \\\n{pad}sha256  {SHA256} \\\n{pad}size    3"
        )
    );
    let missing = ["c".to_string()];
    assert!(
        checksum::verify(root.path(), &missing, &table)
            .unwrap_err()
            .to_string()
            .starts_with("c does not exist in ")
    );
}

fn session(root: &std::path::Path, script: &str) -> Session {
    Session::open(
        &format!("name app; version 1; {script}"),
        &BTreeMap::from([("distpath".into(), root.to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}

#[test]
fn session_verifies_live_distfiles_and_honours_skip() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("app-1.tar.gz"), "abc").unwrap();
    let mut s = session(root.path(), &format!("checksums sha256 {SHA256} size 3"));
    s.verify_source_checksums().unwrap();
    s.invoke("set", &["all_dist_files", "app-1.tar.gz"])
        .unwrap();
    s.verify_source_checksums().unwrap();
    s.invoke("checksums", &["rmd160", "0"]).unwrap();
    let error = s.verify_source_checksums().unwrap_err().to_string();
    assert_eq!(error, "Unable to verify file checksums");
    s.invoke("checksums", &["broken"]).unwrap();
    assert!(s.verify_source_checksums().is_err());
    s.invoke("checksums", &["size", "3"]).unwrap();
    fs::write(
        root.path().join("app-1.tar.gz"),
        "<html><body>moved</body></html>",
    )
    .unwrap();
    assert!(s.verify_source_checksums().is_err());
    assert!(root.path().join("app-1.tar.gz.html").is_file());
    assert!(!root.path().join("app-1.tar.gz").exists());
    s.invoke("checksum.skip", &["yes"]).unwrap();
    s.verify_source_checksums().unwrap();
}
