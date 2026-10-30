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
        vec!["installed", "active", "and", "requested"],
        vec!["installed", "name:curl"],
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
