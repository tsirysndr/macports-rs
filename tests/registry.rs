use rusqlite::Connection;
use std::{
    path::PathBuf,
    process::{Command, Output},
};
struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let db = Connection::open(&path).unwrap();
        db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
        db.execute_batch("CREATE TABLE ports (id INTEGER PRIMARY KEY, name TEXT COLLATE NOCASE, version TEXT COLLATE VERSION, revision INTEGER, variants TEXT, state TEXT, requested INTEGER, requested_variants TEXT, os_platform TEXT, os_major INTEGER, archs TEXT, date INTEGER);
            CREATE INDEX port_name ON ports(name, version, revision, variants);
            INSERT INTO ports VALUES (1,'pkg10','1.0',0,'','imaged',0,'','darwin',24,'arm64',0);
            INSERT INTO ports VALUES (2,'curl','8.9',1,'+ssl','imaged',1,'+ssl','darwin',24,'arm64',0);
            INSERT INTO ports VALUES (3,'curl','8.10',0,'+ssl','installed',1,'+ssl','darwin',24,'arm64',0);
            INSERT INTO ports VALUES (4,'pkg2','2.0',0,'','installed',0,'','darwin',24,'arm64',0);
            INSERT INTO ports VALUES (5,'unfinished','1',0,'','partial',0,'','darwin',24,'arm64',0);").unwrap();
        Self { dir, path }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .env("PORT_RS_REGISTRY", &self.path)
            .env("PORT_RS_INDEX", self.dir.path().join("missing-index"))
            .env("HOME", self.dir.path().join("missing-home"))
            .env("PATH", "")
            .env("NO_COLOR", "1")
            .env("TZ", "UTC")
            .current_dir(self.dir.path())
            .output()
            .unwrap()
    }
}
fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn lists_active_and_inactive_images_in_name_and_version_order() {
    let f = Fixture::new();
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        stdout(f.run(&["installed"])),
        "The following ports are currently installed:\n  curl @8.9_1+ssl\n  curl @8.10_0+ssl (active)\n  pkg2 @2.0_0 (active)\n  pkg10 @1.0_0\n"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert!(!f.dir.path().join("missing-home").exists());
}
#[test]
fn selects_names_versions_variants_and_globs() {
    let f = Fixture::new();
    for args in [
        vec!["-q", "installed", "CURL", "@8.9"],
        vec!["-q", "installed", "curl@8.9_1+ssl"],
    ] {
        assert_eq!(stdout(f.run(&args)), "  curl @8.9_1+ssl\n");
    }
    assert_eq!(
        stdout(f.run(&["-q", "installed", "pkg*", "pkg2"])),
        "  pkg2 @2.0_0 (active)\n  pkg10 @1.0_0\n"
    );
    assert!(!f.run(&["installed", "curl@8.9_1"]).status.success());
}
#[test]
fn supports_state_and_requested_selectors() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "inactive"])),
        "  curl @8.9_1+ssl\n  pkg10 @1.0_0\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "active"])),
        "  curl @8.10_0+ssl (active)\n  pkg2 @2.0_0 (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "requested"])),
        "  curl @8.9_1+ssl\n  curl @8.10_0+ssl (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "unrequested"])),
        "  pkg2 @2.0_0 (active)\n  pkg10 @1.0_0\n"
    );
}
#[test]
fn empty_and_missing_selections_are_distinct() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["installed", "absent*"])),
        "None of the specified ports are installed.\n"
    );
    assert_eq!(stdout(f.run(&["-q", "installed", "absent*"])), "");
    let output = f.run(&["installed", "curl", "missing"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not registered as installed"));
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute("DELETE FROM ports", []).unwrap();
    assert_eq!(stdout(f.run(&["installed"])), "No ports are installed.\n");
}
#[cfg(target_os = "macos")]
#[test]
fn verbose_metadata_and_dates_match_macports_format() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["-q", "-v", "installed", "curl@8.10"])),
        "  curl @8.10_0+ssl (active) requested_variants='+ssl' platform='darwin 24' archs='arm64' date='1970-01-01T00:00:00+0000'\n"
    );
}
#[test]
fn missing_corrupt_and_wrong_schema_registries_fail_without_creation() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    assert!(!f.run(&["installed"]).status.success());
    assert!(!f.path.exists());
    std::fs::write(&f.path, "not a database").unwrap();
    assert!(!f.run(&["installed"]).status.success());
    std::fs::remove_file(&f.path).unwrap();
    Connection::open(&f.path)
        .unwrap()
        .execute_batch("CREATE TABLE ports (name TEXT)")
        .unwrap();
    let output = f.run(&["installed"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported registry schema"));
}
#[test]
fn reads_committed_wal_entries_while_writer_connection_is_open() {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; INSERT INTO ports (name,version,revision,variants,state) VALUES ('wal-port','1',0,'','installed');").unwrap();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "wal-port"])),
        "  wal-port @1_0 (active)\n"
    );
}
#[test]
fn registry_override_chains_and_unsupported_expressions_are_explicit() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "--registry-file",
            f.path.to_str().unwrap(),
            "installe",
            "pkg2",
            ";",
            "installed",
            "pkg10"
        ])),
        "  pkg2 @2.0_0 (active)\n  pkg10 @1.0_0\n"
    );
    for args in [
        vec!["installed", "name:curl"],
        vec!["installed", "category:net"],
    ] {
        let output = f.run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
    }
}

#[test]
fn older_registries_without_optional_metadata_are_readable() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    Connection::open(&f.path).unwrap().execute_batch("CREATE TABLE ports(name TEXT,version TEXT,revision INTEGER,variants TEXT,state TEXT); INSERT INTO ports VALUES('legacy','1',2,NULL,'installed');").unwrap();
    assert_eq!(
        stdout(f.run(&["-q", "-v", "installed"])),
        "  legacy @1_2 (active)\n"
    );
}

#[test]
fn uncommitted_registry_changes_are_not_visible() {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; BEGIN; DELETE FROM ports;")
        .unwrap();
    assert!(stdout(f.run(&["-q", "installed", "active"])).contains("curl @8.10_0+ssl (active)"));
    db.execute_batch("ROLLBACK").unwrap();
}

fn files_fixture() -> Fixture {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch(
        "CREATE TABLE files(id INTEGER, path TEXT, actual_path TEXT, active INTEGER);
        INSERT INTO files VALUES (2,'/opt/local/old-curl',NULL,0);
        INSERT INTO files VALUES (3,'/opt/local/original','/opt/local/renamed-curl',1);
        INSERT INTO files VALUES (3,'/opt/local/hidden','/opt/local/hidden',0);
        INSERT INTO files VALUES (3,'/opt/local/a','/opt/local/a',1);
        INSERT INTO files VALUES (4,'/opt/local/pkg2','/opt/local/pkg2',1);",
    )
    .unwrap();
    f
}
#[test]
fn contents_prefers_active_image_and_lists_actual_paths_in_order() {
    let f = files_fixture();
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        stdout(f.run(&["contents", "CURL"])),
        "Port curl @8.10_0+ssl contains:\n  /opt/local/a\n  /opt/local/renamed-curl\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "contents", "curl@8.9"])),
        "  /opt/local/old-curl\n"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}
#[test]
fn contents_reports_ambiguous_inactive_versions_and_empty_images() {
    let f = files_fixture();
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute("UPDATE ports SET state='imaged' WHERE name='curl'", [])
        .unwrap();
    let output = f.run(&["contents", "curl"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("multiple inactive versions"));
    assert!(error.contains("@8.9_1+ssl"));
    assert!(error.contains("@8.10_0+ssl"));
    assert_eq!(
        stdout(f.run(&["contents", "pkg10"])),
        "Port pkg10 @1.0_0 does not have any files registered.\n"
    );
    assert_eq!(stdout(f.run(&["-q", "contents", "pkg10"])), "");
}
#[test]
fn contents_validates_all_roots_before_printing() {
    let f = files_fixture();
    let output = f.run(&["contents", "pkg2", "missing"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    for selector in ["active", "pkg*"] {
        let output = f.run(&["contents", selector]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
    }
}
#[test]
fn contents_handles_missing_files_table_as_an_error() {
    let f = Fixture::new();
    let output = f.run(&["contents", "curl"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read files for curl"));
}
#[test]
fn registry_entry_and_file_queries_share_a_snapshot() {
    let f = files_fixture();
    let writer = Connection::open(&f.path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    let reader = port_rs::registry::Registry::open(Some(&f.path))
        .unwrap()
        .unwrap();
    let entries = reader.entries().unwrap();
    let curl = entries
        .iter()
        .find(|entry| entry.name == "curl" && entry.active)
        .unwrap();
    writer
        .execute(
            "UPDATE files SET actual_path='/changed' WHERE id=3 AND active",
            [],
        )
        .unwrap();
    assert_eq!(
        reader.files(curl).unwrap(),
        ["/opt/local/a", "/opt/local/renamed-curl"]
    );
    let fresh = port_rs::registry::Registry::open(Some(&f.path))
        .unwrap()
        .unwrap();
    assert_eq!(fresh.files(curl).unwrap(), ["/changed", "/changed"]);
}
#[cfg(unix)]
#[test]
fn contents_sizes_follow_symlinks_and_report_missing_paths() {
    let f = files_fixture();
    let file = f.dir.path().join("file");
    std::fs::write(&file, vec![0u8; 1536]).unwrap();
    let link = f.dir.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let missing = f.dir.path().join("missing");
    let db = Connection::open(&f.path).unwrap();
    db.execute("DELETE FROM files WHERE id=4", []).unwrap();
    for path in [&file, &link, &missing] {
        db.execute(
            "INSERT INTO files VALUES(4,?1,?1,1)",
            [path.to_str().unwrap()],
        )
        .unwrap();
    }
    let output = stdout(f.run(&["-q", "contents", "--size", "--units", "Ki", "pkg2"]));
    assert_eq!(
        output,
        format!(
            "{:>12} {}\n{:>12} {}\n{:>12} {}\n",
            "1.50 KiB",
            file.display(),
            "1.50 KiB",
            link.display(),
            "-1",
            missing.display()
        )
    );
}

fn register_path(f: &Fixture, path: &std::path::Path, active: bool) {
    Connection::open(&f.path)
        .unwrap()
        .execute(
            "INSERT INTO files(id,path,actual_path,active) VALUES(3,?1,?1,?2)",
            rusqlite::params![path.to_str().unwrap(), active],
        )
        .unwrap();
}
#[test]
fn provides_reports_active_owners_and_ignores_inactive_file_records() {
    let f = files_fixture();
    let directory = f.dir.path().canonicalize().unwrap();
    let owned = directory.join("owned");
    let inactive = directory.join("inactive");
    std::fs::write(&owned, "file").unwrap();
    std::fs::write(&inactive, "file").unwrap();
    register_path(&f, &owned, true);
    register_path(&f, &inactive, false);
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        stdout(f.run(&[
            "provides",
            owned.to_str().unwrap(),
            inactive.to_str().unwrap()
        ])),
        format!(
            "{} is provided by: curl\n{} is not provided by a MacPorts port.\n",
            owned.display(),
            inactive.display()
        )
    );
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "provides",
            owned.to_str().unwrap(),
            inactive.to_str().unwrap()
        ])),
        "curl\n"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}
#[test]
fn provides_distinguishes_missing_paths_directories_and_unowned_files() {
    let f = files_fixture();
    let directory = f.dir.path().canonicalize().unwrap();
    let missing = directory.join("missing");
    let unowned = directory.join("unowned");
    std::fs::write(&unowned, "").unwrap();
    assert_eq!(
        stdout(f.run(&[
            "provides",
            missing.to_str().unwrap(),
            directory.to_str().unwrap(),
            unowned.to_str().unwrap()
        ])),
        format!(
            "{} does not exist.\n{} is a directory.\n{} is not provided by a MacPorts port.\n",
            missing.display(),
            directory.display(),
            unowned.display()
        )
    );
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "provides",
            missing.to_str().unwrap(),
            directory.to_str().unwrap(),
            unowned.to_str().unwrap()
        ])),
        ""
    );
}
#[cfg(unix)]
#[test]
fn provides_normalizes_parents_and_preserves_final_symlink_ownership() {
    let f = files_fixture();
    let directory = f.dir.path().canonicalize().unwrap();
    let real = directory.join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = directory.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let target = real.join("file");
    std::fs::write(&target, "").unwrap();
    register_path(&f, &target, true);
    let dangling = directory.join("dangling");
    std::os::unix::fs::symlink("absent-target", &dangling).unwrap();
    register_path(&f, &dangling, true);
    let dirlink = directory.join("dirlink");
    std::os::unix::fs::symlink(&real, &dirlink).unwrap();
    register_path(&f, &dirlink, true);
    assert_eq!(
        stdout(f.run(&["provides", "./alias/file", "./dangling", "dirlink"])),
        format!(
            "{} is provided by: curl\n{} is provided by: curl\n{} is provided by: curl\n",
            target.display(),
            dangling.display(),
            dirlink.display()
        )
    );
    assert_eq!(
        port_rs::registry::path::normalize(&alias.join("../dangling")).unwrap(),
        dangling
    );
}
#[test]
fn owner_queries_bind_paths_and_obey_explicit_case_rules() {
    let f = files_fixture();
    let directory = f.dir.path().canonicalize().unwrap();
    let quoted = directory.join("quoted'; SELECT 'name");
    std::fs::write(&quoted, "").unwrap();
    register_path(&f, &quoted, true);
    assert_eq!(
        stdout(f.run(&["-q", "provides", quoted.to_str().unwrap()])),
        "curl\n"
    );
    let mixed = directory.join("MixedCase");
    register_path(&f, &mixed, true);
    let registry = port_rs::registry::Registry::open(Some(&f.path))
        .unwrap()
        .unwrap();
    let lower = directory.join("mixedcase");
    assert_eq!(registry.owner(lower.to_str().unwrap(), true).unwrap(), None);
    assert_eq!(
        registry.owner(lower.to_str().unwrap(), false).unwrap(),
        Some("curl".into())
    );
}
#[test]
fn provides_uses_registered_actual_path_and_supports_command_chains() {
    let f = files_fixture();
    let directory = f.dir.path().canonicalize().unwrap();
    let original = directory.join("original");
    let renamed = directory.join("renamed");
    std::fs::write(&original, "").unwrap();
    std::fs::write(&renamed, "").unwrap();
    Connection::open(&f.path)
        .unwrap()
        .execute(
            "INSERT INTO files VALUES(3,?1,?2,1)",
            rusqlite::params![original.to_str().unwrap(), renamed.to_str().unwrap()],
        )
        .unwrap();
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "prov",
            original.to_str().unwrap(),
            ";",
            "prov",
            renamed.to_str().unwrap()
        ])),
        "curl\n"
    );
}
#[test]
fn provides_reports_database_errors_without_partial_results() {
    let f = Fixture::new();
    let directory = f.dir.path().canonicalize().unwrap();
    let file = directory.join("file");
    std::fs::write(&file, "").unwrap();
    let output = f.run(&["provides", "missing", file.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot look up registry owner"));
}

fn dependent_fixture(records: &[(&str, &[&str])]) -> Fixture {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute_batch(
        "BEGIN; DELETE FROM ports; CREATE TABLE dependencies(id INTEGER,name TEXT,variants TEXT);",
    )
    .unwrap();
    for (i, (name, deps)) in records.iter().enumerate() {
        let id = i as i64 + 1;
        db.execute("INSERT INTO ports(id,name,version,revision,variants,state) VALUES(?1,?2,'1',0,'','installed')",rusqlite::params![id,name]).unwrap();
        for dep in *deps {
            db.execute(
                "INSERT INTO dependencies(id,name) VALUES(?1,?2)",
                rusqlite::params![id, dep],
            )
            .unwrap();
        }
    }
    db.execute_batch("COMMIT").unwrap();
    f
}
#[test]
fn dependents_use_registry_edges_and_deduplicate_names_unless_verbose() {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch("CREATE TABLE dependencies(id INTEGER,name TEXT); INSERT INTO dependencies VALUES(2,'PKG2'),(3,'pkg2'),(1,'pkg2');").unwrap();
    assert_eq!(
        stdout(f.run(&["dependents", "pkg2"])),
        "curl depends on pkg2\npkg10 depends on pkg2\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "-v", "dependents", "pkg2"])),
        "curl\npkg10\n"
    );
    assert_eq!(
        stdout(f.run(&["-v", "dependents", "pkg2"])),
        "curl @8.9_1+ssl depends on pkg2 (by port:)\ncurl @8.10_0+ssl depends on pkg2 (by port:)\npkg10 @1.0_0 depends on pkg2 (by port:)\n"
    );
}
#[test]
fn recursive_dependents_deduplicate_shared_subtrees_and_full_repeats_versions() {
    let f = dependent_fixture(&[
        ("base", &[]),
        ("left", &["base"]),
        ("right", &["base"]),
        ("top", &["left", "right"]),
    ]);
    assert_eq!(
        stdout(f.run(&["rdependents", "base"])),
        "The following ports are dependent on base:\n  left\n    top\n  right\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "rdependents", "--full", "base"])),
        "  left @1_0\n    top @1_0\n  right @1_0\n    top @1_0\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "-v", "rdependents", "base"])),
        "  left @1_0\n    top @1_0\n  right @1_0\n"
    );
}
#[test]
fn recursive_registry_cycles_terminate_in_both_modes() {
    let f = dependent_fixture(&[("a", &["b"]), ("b", &["a"])]);
    assert_eq!(stdout(f.run(&["-q", "rdependents", "a"])), "  b\n    a\n");
    assert_eq!(
        stdout(f.run(&["-q", "rdependents", "--full", "a"])),
        "  b @1_0\n    a @1_0\n      b @1_0 (cyclic dependency)\n"
    );
}
#[test]
fn dependent_queries_handle_empty_missing_and_qualified_roots() {
    let f = dependent_fixture(&[("base", &[]), ("child", &["base"]), ("isolated", &[])]);
    assert_eq!(
        stdout(f.run(&["dependents", "BASE@1_0"])),
        "child depends on base\n"
    );
    for action in ["dependents", "rdependents"] {
        assert_eq!(
            stdout(f.run(&[action, "isolated"])),
            "isolated has no dependents.\n"
        );
        assert_eq!(stdout(f.run(&["-q", action, "isolated"])), "");
        for missing in ["absent", "base@99"] {
            let output = f.run(&[action, "base", missing]);
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}
#[test]
fn direct_dependents_deduplicate_across_requested_roots() {
    let f = dependent_fixture(&[("a", &[]), ("b", &[]), ("child", &["a", "b"])]);
    assert_eq!(
        stdout(f.run(&["dependents", "a", "b"])),
        "child depends on a\n"
    );
    assert_eq!(
        stdout(f.run(&["-v", "dependents", "a", "b"])),
        "child @1_0 depends on a (by port:)\nchild @1_0 depends on b (by port:)\n"
    );
}
#[test]
fn dependent_query_schema_errors_are_explicit() {
    let f = Fixture::new();
    let output = f.run(&["dependents", "curl"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read registry dependencies"));
}
#[test]
fn registry_dependent_walk_handles_deep_graphs_iteratively() {
    let names: Vec<_> = (0..5000).map(|i| format!("node{i}")).collect();
    let deps: Vec<Vec<&str>> = (0..5000)
        .map(|i| {
            if i == 0 {
                vec![]
            } else {
                vec![names[i - 1].as_str()]
            }
        })
        .collect();
    let records: Vec<_> = names
        .iter()
        .zip(&deps)
        .map(|(name, deps)| (name.as_str(), deps.as_slice()))
        .collect();
    let f = dependent_fixture(&records);
    let registry = port_rs::registry::Registry::open(Some(&f.path))
        .unwrap()
        .unwrap();
    let entries = registry.entries().unwrap();
    let graph = port_rs::registry::dependents::Graph::build(&registry, &entries).unwrap();
    let mut count = 0;
    let mut deepest = 0;
    graph
        .walk("node0", false, |_, depth, cycle| {
            assert!(!cycle);
            count += 1;
            deepest = depth;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 4999);
    assert_eq!(deepest, 4999);
    drop(graph);
    drop(registry);
    Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE ports SET requested=1 WHERE id=5000", [])
        .unwrap();
    assert_eq!(stdout(f.run(&["-q", "installed", "rleaves"])), "");
}

fn outdated_fixture(records: &[(&str, &str)]) -> Fixture {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch(
        "ALTER TABLE ports ADD COLUMN epoch INTEGER DEFAULT 0; UPDATE ports SET os_platform='any';",
    )
    .unwrap();
    let mut data = String::new();
    for (name, fields) in records {
        let body = format!("name {name} {fields}");
        data.push_str(&format!(
            "{name} {}\n{body}\n",
            body.encode_utf16().count() + 1
        ));
    }
    std::fs::write(f.dir.path().join("missing-index"), data).unwrap();
    f
}
#[test]
fn outdated_compares_only_active_versions_and_revisions() {
    let f = outdated_fixture(&[
        ("curl", "version 8.11 revision 0"),
        ("pkg2", "version 2.0 revision 1"),
        ("pkg10", "version 99 revision 0"),
    ]);
    let output = stdout(f.run(&["outdated"]));
    assert!(output.starts_with("The following installed ports are outdated:\n"));
    assert!(output.contains("8.10_0 < 8.11_0"));
    assert!(output.contains("2.0_0 < 2.0_1"));
    assert!(!output.contains("8.9_1"));
    assert!(!output.contains("pkg10"));
    let quiet = stdout(f.run(&["-q", "outdated", "CURL@8.10"]));
    assert_eq!(
        quiet,
        format!("{:<30} {:<24}  \n", "curl", "8.10_0 < 8.11_0 ")
    );
}
#[test]
fn verbose_outdated_reports_newer_installed_versions_with_flag() {
    let f = outdated_fixture(&[("curl", "version 8.1 revision 0"), ("pkg2", "version 2.0")]);
    assert_eq!(
        stdout(f.run(&["outdated"])),
        "No installed ports are outdated.\n"
    );
    let output = stdout(f.run(&["-q", "-v", "outdated"]));
    assert!(output.contains("8.10_0 > 8.1_0"));
    assert!(output.trim_end().ends_with('!'));
}
#[test]
fn outdated_epoch_overrides_version_only_when_version_strings_differ() {
    let f = outdated_fixture(&[("curl", "version 7 epoch 1 revision 0")]);
    let output = stdout(f.run(&["-q", "outdated", "curl"]));
    assert!(output.contains("8.10_0 < 7_0  (epoch 0 < 1)"));
    let f = outdated_fixture(&[("curl", "version 8.10 epoch 99 revision 0")]);
    assert_eq!(
        stdout(f.run(&["outdated", "curl"])),
        "No installed ports are outdated.\n"
    );
    let f = outdated_fixture(&[("curl", "version 9 epoch 0 revision 0")]);
    Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE ports SET epoch=1 WHERE name='curl'", [])
        .unwrap();
    assert!(
        stdout(f.run(&["-q", "-v", "outdated", "curl"])).contains("8.10_0 > 9_0  (epoch 1 > 0)")
    );
}
#[cfg(target_os = "macos")]
#[test]
fn outdated_identifies_platform_rebuilds_for_otherwise_equal_versions() {
    let f = outdated_fixture(&[("curl", "version 8.10 revision 0")]);
    Connection::open(&f.path)
        .unwrap()
        .execute(
            "UPDATE ports SET os_platform='darwin',os_major=1 WHERE name='curl'",
            [],
        )
        .unwrap();
    let major = port_rs::source::detect_platform().unwrap().darwin_major;
    assert!(
        stdout(f.run(&["-q", "outdated", "curl"])).contains(&format!(
            "8.10_0 < 8.10_0  (platform darwin 1 != darwin {major})"
        ))
    );
}
#[test]
fn outdated_skips_removed_ports_and_warns_about_missing_version_metadata() {
    let f = outdated_fixture(&[("curl", "description unavailable")]);
    let output = f.run(&["outdated"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("curl has no version field"));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "No installed ports are outdated.\n"
    );
}
#[test]
fn empty_outdated_selections_do_not_initialize_the_catalog() {
    let f = Fixture::new();
    for selector in ["missing", "inactive", "curl@99"] {
        assert_eq!(
            stdout(f.run(&["outdated", selector])),
            "None of the specified ports are outdated.\n"
        );
    }
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute("DELETE FROM ports", []).unwrap();
    assert_eq!(stdout(f.run(&["outdated"])), "No ports are installed.\n");
    assert!(!f.dir.path().join("missing-home").exists());
}
#[test]
fn outdated_rejects_bad_numeric_metadata_before_printing_results() {
    let f = outdated_fixture(&[
        ("curl", "version 99"),
        ("pkg2", "version 99 revision broken"),
    ]);
    let output = f.run(&["outdated"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("pkg2: outdated comparison"));
}
#[test]
fn outdated_globs_and_requested_filters_compose_with_command_chains() {
    let f = outdated_fixture(&[("curl", "version 99"), ("pkg2", "version 99")]);
    let output = stdout(f.run(&["-q", "outdated", "requested", ";", "outdated", "pkg*"]));
    let names: Vec<_> = output
        .lines()
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names, ["curl", "pkg2"]);
}

#[test]
fn installed_logical_expressions_obey_precedence_and_parentheses() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "active", "and", "requested"])),
        "  curl @8.10_0+ssl (active)\n"
    );
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "installed",
            "inactive",
            "or",
            "active",
            "and",
            "requested"
        ])),
        "  curl @8.9_1+ssl\n  curl @8.10_0+ssl (active)\n  pkg10 @1.0_0\n"
    );
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "installed",
            "(",
            "inactive",
            "or",
            "active",
            ")",
            "and",
            "requested"
        ])),
        "  curl @8.9_1+ssl\n  curl @8.10_0+ssl (active)\n"
    );
}
#[test]
fn registry_complements_are_scoped_to_installed_images() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "not", "active"])),
        "  curl @8.9_1+ssl\n  pkg10 @1.0_0\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "!", "requested", "and", "active"])),
        "  pkg2 @2.0_0 (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "not", "(", "installed", ")"])),
        ""
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "not", "absent*"])),
        stdout(f.run(&["-q", "installed"]))
    );
}
#[test]
fn registry_set_operations_preserve_distinct_versions_and_variants() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "curl", "and", "not", "curl@8.9"])),
        "  curl @8.10_0+ssl (active)\n"
    );
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "installed",
            "curl@8.9_1+ssl",
            "or",
            "curl@8.10_0+ssl",
            "curl@8.9"
        ])),
        "  curl @8.9_1+ssl\n  curl @8.10_0+ssl (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "curl@8.9", "and", "curl@8.10"])),
        ""
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "curl", "and", "curl", "@8.10_0", "+ssl"])),
        "  curl @8.10_0+ssl (active)\n"
    );
}
#[test]
fn malformed_registry_expressions_fail_without_partial_output() {
    let f = Fixture::new();
    for expr in [
        vec!["active", "and"],
        vec!["(", "active"],
        vec!["active", ")"],
        vec!["(", ")"],
        vec!["or", "active"],
        vec!["not"],
        vec!["+ssl"],
    ] {
        let mut args = vec!["installed"];
        args.extend(expr);
        let output = f.run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
    let mut args = vec!["installed"];
    args.extend(std::iter::repeat_n("not", 130));
    args.push("active");
    let output = f.run(&args);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nesting exceeds 128"));
}
#[test]
fn outdated_logical_expressions_filter_images_before_comparison() {
    let f = outdated_fixture(&[("curl", "version 99"), ("pkg2", "version 99")]);
    let output = stdout(f.run(&[
        "-q",
        "outdated",
        "(",
        "curl",
        "or",
        "pkg*",
        ")",
        "and",
        "not",
        "requested",
    ]));
    let names: Vec<_> = output
        .lines()
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names, ["pkg2"]);
    assert_eq!(
        stdout(f.run(&["-q", "outdated", "not", "(", "active", ")"])),
        ""
    );
    assert_eq!(
        stdout(f.run(&["-q", "outdated", "missing", "or", "curl"])),
        stdout(f.run(&["-q", "outdated", "curl"]))
    );
}
#[test]
fn logical_registry_selections_work_in_command_chains() {
    let f = Fixture::new();
    assert_eq!(
        stdout(f.run(&[
            "-q",
            "installed",
            "active",
            "and",
            "requested",
            ";",
            "installed",
            "inactive",
            "and",
            "unrequested"
        ])),
        "  curl @8.10_0+ssl (active)\n  pkg10 @1.0_0\n"
    );
}

fn space_fixture() -> Fixture {
    let f = files_fixture();
    let file = f.dir.path().join("payload");
    let hardlink = f.dir.path().join("hardlink");
    let link = f.dir.path().join("dangling");
    let missing = f.dir.path().join("removed");
    std::fs::write(&file, vec![0u8; 4096]).unwrap();
    std::fs::hard_link(&file, &hardlink).unwrap();
    std::os::unix::fs::symlink("absent", &link).unwrap();
    let db = Connection::open(&f.path).unwrap();
    db.execute("DELETE FROM files", []).unwrap();
    for (id, path, active) in [
        (3, &file, 1),
        (3, &hardlink, 1),
        (3, &link, 1),
        (3, &missing, 1),
        (4, &hardlink, 1),
        (3, &file, 0),
    ] {
        db.execute(
            "INSERT INTO files VALUES (?1, '/original-path', ?2, ?3)",
            rusqlite::params![id, path.to_str().unwrap(), active],
        )
        .unwrap();
    }
    f
}
#[test]
fn space_counts_hardlinks_once_per_port_and_lstats_dangling_links() {
    let f = space_fixture();
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        stdout(f.run(&["space", "--units", "B", "curl", "pkg2"])),
        "4102 B curl\n4096 B pkg2\n8198 B total\n"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert!(!f.dir.path().join("missing-home").exists());
}
#[test]
fn space_supports_total_units_quiet_and_action_abbreviations() {
    let f = space_fixture();
    assert_eq!(
        stdout(f.run(&["-q", "spac", "--units", "Ki", "pkg2"])),
        "4.0 KiB pkg2\n"
    );
    assert_eq!(
        stdout(f.run(&["space", "--total", "--units", "B", "curl", "pkg2"])),
        "8198 B total\n"
    );
    assert_eq!(
        stdout(f.run(&["space", "--total", "pkg2"])),
        "4.0 KiB total\n"
    );
}
#[test]
fn space_reports_active_version_even_when_an_inactive_version_is_requested() {
    let f = space_fixture();
    let result = f.run(&["space", "--units", "B", "curl@8.9_1+ssl"]);
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("Active version of curl is not 8.9_1 but 8.10_0")
    );
    assert_eq!(stdout(result), "4102 B curl @8.10_0+ssl\n");
    let result = f.run(&["space", "--units", "B", "curl@8.10_0+ssl"]);
    assert!(result.stderr.is_empty());
    assert_eq!(stdout(result), "4102 B curl @8.10_0+ssl\n");
}
#[test]
fn space_distinguishes_inactive_empty_and_removed_files() {
    let f = space_fixture();
    let db = Connection::open(&f.path).unwrap();
    db.execute("DELETE FROM files WHERE id=4", []).unwrap();
    let result = f.run(&["space", "--total", "pkg10", "pkg2", "missing"]);
    let errors = String::from_utf8_lossy(&result.stderr);
    assert!(errors.contains("Port pkg10 is not active."));
    assert!(errors.contains("Port pkg2 does not have any files registered."));
    assert!(errors.contains("Port missing is not active."));
    assert_eq!(stdout(result), "0 B total\n");
    db.execute(
        "UPDATE files SET actual_path='/nonexistent-port-rs-file'",
        [],
    )
    .unwrap();
    assert_eq!(stdout(f.run(&["space", "curl"])), "0 B curl\n");
}
#[test]
fn space_validates_selectors_and_database_before_output() {
    let f = Fixture::new();
    for args in [
        vec!["space", "curl"],
        vec!["space", "active"],
        vec!["space", "curl", "and", "pkg2"],
        vec!["space", "pkg*"],
    ] {
        let result = f.run(&args);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
}
#[test]
fn space_options_reset_between_chained_commands() {
    let f = space_fixture();
    assert_eq!(
        stdout(f.run(&["space", "--total", "pkg2", ";", "space", "pkg2"])),
        "4.0 KiB total\n4.0 KiB pkg2\n"
    );
}

fn location_fixture() -> Fixture {
    let f = Fixture::new();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch("ALTER TABLE ports ADD COLUMN location TEXT;
        UPDATE ports SET location='/opt/local/var/macports/software/' || name || '/' || name || '-' || version || '.tbz2';
        UPDATE ports SET location='/custom images/pkg2 image' WHERE id=4;").unwrap();
    f
}
#[test]
fn location_prints_recorded_image_paths_without_accessing_them() {
    let f = location_fixture();
    let before = std::fs::read(&f.path).unwrap();
    assert_eq!(
        stdout(f.run(&["location", "PKG2"])),
        "Port pkg2 2.0_0 is installed as an image in:\n/custom images/pkg2 image\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "locat", "pkg2", "pkg10"])),
        "/custom images/pkg2 image\n/opt/local/var/macports/software/pkg10/pkg10-1.0.tbz2\n"
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert!(!f.dir.path().join("missing-home").exists());
}
#[test]
fn location_requires_disambiguation_even_when_one_image_is_active() {
    let f = location_fixture();
    let result = f.run(&["location", "curl"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("specify the full version"));
    assert!(error.contains("@8.9_1+ssl"));
    assert!(error.contains("@8.10_0+ssl (active)"));
    for args in [
        vec!["-q", "location", "curl@8.9"],
        vec!["-q", "location", "curl", "@8.9_1", "+ssl"],
    ] {
        assert_eq!(
            stdout(f.run(&args)),
            "/opt/local/var/macports/software/curl/curl-8.9.tbz2\n"
        );
    }
    assert_eq!(
        stdout(f.run(&["-q", "location", "curl@8.10_0+ssl"])),
        "/opt/local/var/macports/software/curl/curl-8.10.tbz2\n"
    );
}
#[test]
fn location_validates_all_selections_before_output() {
    let f = location_fixture();
    for selector in [
        "missing",
        "curl",
        "curl@8.9_1",
        "active",
        "pkg*",
        "name:pkg2",
    ] {
        let result = f.run(&["location", "pkg2", selector]);
        assert!(!result.status.success(), "{selector}");
        assert!(result.stdout.is_empty(), "{selector}");
    }
}
#[test]
fn location_missing_or_null_column_errors_do_not_affect_other_queries() {
    let f = Fixture::new();
    let result = f.run(&["location", "pkg2"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot read image location"));
    assert!(f.run(&["installed", "pkg2"]).status.success());
    let f = location_fixture();
    Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE ports SET location=NULL WHERE id=1", [])
        .unwrap();
    let result = f.run(&["location", "pkg2", "pkg10"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}
#[test]
fn location_reads_the_same_snapshot_as_entry_selection() {
    let f = location_fixture();
    let writer = Connection::open(&f.path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    let reader = port_rs::registry::Registry::open(Some(&f.path))
        .unwrap()
        .unwrap();
    let entries = reader.entries().unwrap();
    let entry = entries.iter().find(|entry| entry.name == "pkg2").unwrap();
    writer
        .execute("UPDATE ports SET location='/new-image' WHERE id=4", [])
        .unwrap();
    assert_eq!(reader.location(entry).unwrap(), "/custom images/pkg2 image");
    assert_eq!(stdout(f.run(&["-q", "location", "pkg2"])), "/new-image\n");
}
#[test]
fn location_chains_with_registry_commands_and_stops_on_errors() {
    let f = location_fixture();
    assert_eq!(
        stdout(f.run(&["-q", "location", "pkg2", ";", "installed", "pkg2"])),
        "/custom images/pkg2 image\n  pkg2 @2.0_0 (active)\n"
    );
    let result = f.run(&["location", "missing", ";", "installed", "pkg2"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn leaf_selectors_distinguish_direct_and_recursive_orphans() {
    let f = dependent_fixture(&[
        ("app", &["lib"]),
        ("lib", &["base"]),
        ("base", &[]),
        ("orphan", &["unused"]),
        ("unused", &[]),
    ]);
    Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE ports SET requested=1 WHERE name='app'", [])
        .unwrap();
    assert_eq!(
        stdout(f.run(&["-q", "installed", "leaves"])),
        "  orphan @1_0 (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "rleaves"])),
        "  orphan @1_0 (active)\n  unused @1_0 (active)\n"
    );
    assert_eq!(
        stdout(f.run(&["-q", "installed", "rleaves", "and", "not", "leaves"])),
        "  unused @1_0 (active)\n"
    );
    assert_eq!(stdout(f.run(&["-q", "installed", "leaves@99"])), "");
}
#[test]
fn recursive_leaves_handle_cycles_and_requested_inactive_images() {
    let f = dependent_fixture(&[
        ("app", &["LIB"]),
        ("lib", &["base"]),
        ("base", &["lib"]),
        ("cycle1", &["cycle2"]),
        ("cycle2", &["cycle1"]),
    ]);
    Connection::open(&f.path)
        .unwrap()
        .execute(
            "UPDATE ports SET requested=1,state='imaged' WHERE name='app'",
            [],
        )
        .unwrap();
    assert_eq!(stdout(f.run(&["-q", "installed", "leaves"])), "");
    assert_eq!(
        stdout(f.run(&["-q", "installed", "rleaves"])),
        "  cycle1 @1_0 (active)\n  cycle2 @1_0 (active)\n"
    );
}
#[test]
fn leaf_selection_preserves_unrequested_images_of_requested_names() {
    let f = dependent_fixture(&[("app", &[]), ("app", &[])]);
    let db = Connection::open(&f.path).unwrap();
    db.create_collation("VERSION", |a, b| a.cmp(b)).unwrap();
    db.execute_batch("UPDATE ports SET requested=1 WHERE id=1; UPDATE ports SET version='2',state='imaged' WHERE id=2;").unwrap();
    for selector in ["leaves", "rleaves"] {
        assert_eq!(
            stdout(f.run(&["-q", "installed", selector])),
            "  app @2_0\n"
        );
    }
}
#[test]
fn leaf_queries_load_dependencies_only_when_needed() {
    let f = Fixture::new();
    assert!(f.run(&["installed", "active"]).status.success());
    let result = f.run(&["installed", "active", "or", "leaves"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot read registry dependencies"));
}
#[test]
fn outdated_accepts_leaf_expressions_and_excludes_requested_ports() {
    let f = outdated_fixture(&[("curl", "version 99"), ("pkg2", "version 99")]);
    Connection::open(&f.path)
        .unwrap()
        .execute_batch("CREATE TABLE dependencies(id INTEGER,name TEXT);")
        .unwrap();
    let output = stdout(f.run(&["-q", "outdated", "leaves", "or", "rleaves"]));
    assert!(output.contains("pkg2"));
    assert!(!output.contains("curl"));
}
