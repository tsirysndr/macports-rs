use port_rs::install::sites::resource::Resource;

const SOURCE: &str = include_str!("fixtures/archive-sites/archive_sites.tcl");

#[test]
fn installed_resource_retains_mirror_order_tags_layout_and_signature_metadata() {
    let resource = Resource::evaluate(SOURCE, "darwin", 24).unwrap();
    assert_eq!(resource.names().collect::<Vec<_>>(), ["macports_archives"]);
    let urls = resource.urls("macports_archives").unwrap();
    assert_eq!(urls[0], "https://packages.macports.org/:nosubdir");
    assert!(urls.iter().all(|url| url.ends_with(":nosubdir")));
    for (field, expected) in [
        ("archive_prefix", "/opt/local"),
        ("archive_type", "tbz2"),
        ("archive_cxx_stdlib", "libc++"),
        ("archive_delete_la_files", "yes"),
        ("archive_sigtype", "sig"),
        (
            "archive_pubkey",
            "/opt/local/share/macports/keys/archives/macports-archives-2025.pub",
        ),
    ] {
        assert_eq!(resource.value(field, "macports_archives"), Some(expected));
    }
    let old = Resource::evaluate(SOURCE, "darwin", 12).unwrap();
    let old_urls = old.urls("macports_archives").unwrap();
    assert_eq!(old_urls[0], "http://packages.macports.org/:nosubdir");
    assert!(old_urls.iter().all(|url| !url.starts_with(':')));
    assert!(old_urls.len() < urls.len());
    assert_eq!(
        old.value("archive_delete_la_files", "macports_archives"),
        Some("no")
    );
}

#[test]
fn resource_has_no_host_environment_filesystem_output_or_exit_commands() {
    for script in [
        "set env(PATH)",
        "source /etc/passwd",
        "puts injected",
        "exit 0",
    ] {
        assert!(
            Resource::evaluate(script, "darwin", 24).is_err(),
            "{script}"
        );
    }
    let empty = Resource::evaluate("set unrelated 1", "darwin", 24).unwrap();
    assert_eq!(empty.names().count(), 0);
    assert_eq!(empty.value("archive_sigtype", "missing"), None);
    assert!(empty.urls("missing").unwrap().is_empty());
    assert!(Resource::evaluate("proc recurse {} {recurse}; recurse", "darwin", 24).is_err());
}

#[test]
fn resource_file_loading_is_bounded_and_missing_files_are_empty() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("archive_sites.tcl");
    assert_eq!(
        Resource::read(&path, "darwin", 24).unwrap().names().count(),
        0
    );
    std::fs::write(&path, SOURCE).unwrap();
    assert_eq!(
        Resource::read(&path, "darwin", 24).unwrap().names().count(),
        1
    );
    assert!(Resource::read(temp.path(), "darwin", 24).is_err());
    std::fs::write(&path, vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(Resource::read(&path, "darwin", 24).is_err());
    std::fs::write(&path, [0xff]).unwrap();
    assert!(Resource::read(&path, "darwin", 24).is_err());
}
