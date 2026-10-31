use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};
fn session(root: &Path, execution: bool) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::from([
            ("destroot".into(), root.to_str().unwrap().into()),
            ("prefix".into(), "/opt/local".into()),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        s.enable_system(&BTreeMap::new(), |_| {
            panic!("compression must stay in-process")
        })
        .unwrap();
    }
    s
}
fn section(root: &Path) -> PathBuf {
    let path = root.join("opt/local/share/man/man1");
    fs::create_dir_all(&path).unwrap();
    path
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::GzBuilder::new()
        .filename("original-name")
        .mtime(123)
        .write(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
fn decoded(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    flate2::read::MultiGzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}
#[test]
fn normalizes_plain_gzip_and_bzip2_pages_with_deterministic_headers_and_modes() {
    let root = tempfile::tempdir().unwrap();
    let dir = section(root.path());
    let content = b".TH APP 1\nmanual content\n";
    fs::write(dir.join("plain.1"), content).unwrap();
    fs::write(dir.join("compressed.1.gz"), gzip(content)).unwrap();
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    encoder.write_all(content).unwrap();
    fs::write(dir.join("bzip.1.bz2"), encoder.finish().unwrap()).unwrap();
    let timestamp = UNIX_EPOCH + Duration::from_secs(1234567890);
    fs::File::open(dir.join("plain.1"))
        .unwrap()
        .set_modified(timestamp)
        .unwrap();
    let result = session(root.path(), true)
        .compress_destroot_manpages()
        .unwrap();
    assert_eq!(result.compressed, 3);
    for name in ["plain.1.gz", "compressed.1.gz", "bzip.1.gz"] {
        let path = dir.join(name);
        assert_eq!(decoded(&path), content);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..10], &[31, 139, 8, 0, 0, 0, 0, 0, 2, 3]);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o7777,
            0o444
        );
    }
    assert_eq!(
        fs::metadata(dir.join("plain.1.gz"))
            .unwrap()
            .modified()
            .unwrap(),
        timestamp
    );
    assert!(!dir.join("plain.1").exists());
    assert!(!dir.join("bzip.1.bz2").exists());
    let before = fs::read(dir.join("plain.1.gz")).unwrap();
    session(root.path(), true)
        .compress_destroot_manpages()
        .unwrap();
    assert_eq!(fs::read(dir.join("plain.1.gz")).unwrap(), before);
}
#[test]
fn rewrites_relative_absolute_and_chained_links_but_leaves_missing_targets() {
    let root = tempfile::tempdir().unwrap();
    let dir = section(root.path());
    fs::write(dir.join("real.1"), b"manual").unwrap();
    symlink("z-chain.1", dir.join("a-chain.1")).unwrap();
    symlink("real.1", dir.join("z-chain.1")).unwrap();
    symlink("/opt/local/share/man/man1/real.1", dir.join("absolute.1")).unwrap();
    symlink("real.1", dir.join("already.1.gz")).unwrap();
    symlink("missing.1", dir.join("missing.1")).unwrap();
    let result = session(root.path(), true)
        .compress_destroot_manpages()
        .unwrap();
    assert_eq!(result.relinked, 4);
    assert_eq!(
        fs::read_link(dir.join("a-chain.1.gz")).unwrap(),
        Path::new("z-chain.1.gz")
    );
    assert_eq!(
        fs::read_link(dir.join("z-chain.1.gz")).unwrap(),
        Path::new("real.1.gz")
    );
    assert_eq!(
        fs::read_link(dir.join("absolute.1.gz")).unwrap(),
        Path::new("/opt/local/share/man/man1/real.1.gz")
    );
    assert_eq!(
        fs::read_link(dir.join("already.1.gz")).unwrap(),
        Path::new("real.1.gz")
    );
    assert!(fs::symlink_metadata(dir.join("missing.1")).is_ok());
}
#[test]
fn invalid_compressed_input_preserves_source_and_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let dir = section(root.path());
    fs::write(dir.join("bad.1.bz2"), b"not bzip2").unwrap();
    fs::write(dir.join("bad.1.gz"), b"previous destination").unwrap();
    assert!(
        session(root.path(), true)
            .compress_destroot_manpages()
            .is_err()
    );
    assert_eq!(fs::read(dir.join("bad.1.bz2")).unwrap(), b"not bzip2");
    assert_eq!(
        fs::read(dir.join("bad.1.gz")).unwrap(),
        b"previous destination"
    );
    assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
}
#[test]
fn ignores_unrelated_pages_and_physical_directory_links_and_gates_metadata() {
    let root = tempfile::tempdir().unwrap();
    let dir = section(root.path());
    fs::write(dir.join("wrong.2"), "keep").unwrap();
    fs::write(dir.join("README"), "keep").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("external.2"), "keep").unwrap();
    symlink(outside.path(), dir.parent().unwrap().join("man2")).unwrap();
    fs::write(dir.join("valid.1ssl"), "compress").unwrap();
    assert!(
        session(root.path(), false)
            .compress_destroot_manpages()
            .is_err()
    );
    assert!(dir.join("valid.1ssl").exists());
    assert_eq!(
        session(root.path(), true)
            .compress_destroot_manpages()
            .unwrap()
            .compressed,
        1
    );
    assert_eq!(fs::read_to_string(dir.join("wrong.2")).unwrap(), "keep");
    assert!(outside.path().join("external.2").exists());
}
#[test]
fn decodes_concatenated_members_and_refuses_link_name_collisions() {
    let root = tempfile::tempdir().unwrap();
    let dir = section(root.path());
    fs::write(
        dir.join("real.1.gz"),
        [gzip(b"first"), gzip(b"second")].concat(),
    )
    .unwrap();
    symlink("real.1", dir.join("alias.1")).unwrap();
    symlink("missing", dir.join("alias.1.gz")).unwrap();
    assert!(
        session(root.path(), true)
            .compress_destroot_manpages()
            .is_err()
    );
    assert_eq!(decoded(&dir.join("real.1.gz")), b"firstsecond");
    assert_eq!(
        fs::read_link(dir.join("alias.1")).unwrap(),
        Path::new("real.1")
    );
    assert_eq!(
        fs::read_link(dir.join("alias.1.gz")).unwrap(),
        Path::new("missing")
    );
}

#[test]
fn refuses_man_tree_reached_through_an_outward_prefix_link() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("share/man/man1")).unwrap();
    fs::write(outside.path().join("share/man/man1/external.1"), "preserve").unwrap();
    fs::create_dir(root.path().join("opt")).unwrap();
    symlink(outside.path(), root.path().join("opt/local")).unwrap();
    assert!(
        session(root.path(), true)
            .compress_destroot_manpages()
            .unwrap_err()
            .to_string()
            .contains("outside destroot")
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("share/man/man1/external.1")).unwrap(),
        "preserve"
    );
    assert!(!outside.path().join("share/man/man1/external.1.gz").exists());
}
