use rusqlite::Connection;
use std::process::{Command, Output};
struct Fixture {
    dir: tempfile::TempDir,
}
impl Fixture {
    fn new(records: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut index = String::new();
        for (name, fields) in records {
            let body = format!("name {name} {fields}");
            index.push_str(&format!(
                "{name} {}\n{body}\n",
                body.encode_utf16().count() + 1
            ));
        }
        std::fs::write(dir.path().join("PortIndex"), index).unwrap();
        Connection::open(dir.path().join("registry.db")).unwrap().execute_batch("CREATE TABLE ports(id INTEGER PRIMARY KEY,name TEXT,version TEXT,revision INTEGER,variants TEXT,state TEXT);").unwrap();
        Self { dir }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .env("PORT_RS_INDEX", self.dir.path().join("PortIndex"))
            .env("PORT_RS_REGISTRY", self.dir.path().join("registry.db"))
            .env("HOME", self.dir.path().join("home"))
            .env("PATH", "")
            .env("NO_COLOR", "1")
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
fn dry_run_orders_runtime_dependencies_and_ignores_build_dependencies() {
    let f = Fixture::new(&[
        (
            "app",
            "version 1 depends_lib {port:left port:right} depends_build port:missing",
        ),
        ("left", "version 2 depends_run port:base"),
        ("right", "version 3 depends_lib port:base"),
        ("base", "version 4"),
    ]);
    let before = std::fs::read(f.dir.path().join("registry.db")).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  base @4_0 (dependency)\ninstall  left @2_0 (dependency)\ninstall  right @3_0 (dependency)\ninstall  app @1_0 (requested)\n"
    );
    assert_eq!(
        std::fs::read(f.dir.path().join("registry.db")).unwrap(),
        before
    );
    assert!(!f.dir.path().join("home").exists());
    assert_eq!(std::fs::read_dir(f.dir.path()).unwrap().count(), 2);
}
#[test]
fn shared_explicit_roots_remain_requested_and_are_deduplicated() {
    let f = Fixture::new(&[
        ("app", "version 1 depends_run port:base"),
        ("base", "version 2"),
    ]);
    assert_eq!(
        stdout(f.run(&["-q", "install", "--dry-run", "APP", "base", "app"])),
        "install  base @2_0 (requested)\ninstall  app @1_0 (requested)\n"
    );
}
#[test]
fn active_and_inactive_matching_images_have_distinct_actions() {
    let f = Fixture::new(&[
        ("app", "version 1 depends_run port:base"),
        ("base", "version 2"),
    ]);
    let db = Connection::open(f.dir.path().join("registry.db")).unwrap();
    db.execute_batch("INSERT INTO ports VALUES(1,'base','2',0,'','installed'); INSERT INTO ports VALUES(2,'app','1',0,'','imaged')").unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "active   base @2_0 (dependency)\nactivate app @1_0 (requested)\n"
    );
    db.execute("UPDATE ports SET state='installed' WHERE id=2", [])
        .unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "active   app @1_0 (requested)\n"
    );
}
#[test]
fn conflicting_active_images_fail_without_partial_plan() {
    let f = Fixture::new(&[
        ("app", "version 1 depends_run port:base"),
        ("base", "version 2"),
    ]);
    Connection::open(f.dir.path().join("registry.db"))
        .unwrap()
        .execute_batch("INSERT INTO ports VALUES(1,'base','1',0,'','installed')")
        .unwrap();
    let output = f.run(&["-y", "install", "app"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already active with a different"));
}
#[test]
fn missing_dependencies_cycles_and_providers_fail_before_output() {
    for records in [
        vec![("app", "version 1 depends_run port:missing")],
        vec![
            ("app", "version 1 depends_run port:base"),
            ("base", "version 1 depends_run port:app"),
        ],
        vec![
            ("app", "version 1 depends_run bin:base:base"),
            ("base", "version 1"),
        ],
    ] {
        let f = Fixture::new(&records);
        let output = f.run(&["-y", "install", "app"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn only_indexed_versions_and_default_variants_are_planned() {
    let f = Fixture::new(&[(
        "app",
        "version 1 revision 2 variants {ssl extra} vinfo {ssl {is_default +} extra {description optional}}",
    )]);
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app@1_2+ssl-extra"])),
        "install  app @1_2+ssl (requested)\n"
    );
    for spec in ["app@0", "app@1_2+extra", "app@1_2/-ssl", "app@1_2+missing"] {
        let output = f.run(&["-y", "install", spec]);
        assert!(!output.status.success(), "{spec}");
        assert!(output.stdout.is_empty());
    }
    let output = f.run(&["-y", "install", "app", "configure.args=test"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
#[test]
fn insufficient_metadata_and_replaced_ports_are_explicit_errors() {
    for fields in [
        "revision 1",
        "version 1 revision bad",
        "version 1 revision -1",
        "version 1 variants ssl",
        "version 1 replaced_by other",
    ] {
        let f = Fixture::new(&[("app", fields)]);
        let output = f.run(&["-y", "install", "app"]);
        assert!(!output.status.success(), "{fields}");
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn ordinary_install_is_blocked_before_index_or_registry_access() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_port"))
        .args(["install", "app"])
        .env("PORT_RS_INDEX", dir.path().join("missing-index"))
        .env("PORT_RS_REGISTRY", dir.path().join("missing-registry"))
        .env("HOME", dir.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("use -y install"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn install_dry_run_chains_reset_action_flags() {
    let f = Fixture::new(&[("app", "version 1")]);
    let output = f.run(&["-q", "install", "--dry-run", "app", ";", "install", "app"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "install  app @1_0 (requested)\n"
    );
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app", ";", "install", "app"])),
        "install  app @1_0 (requested)\ninstall  app @1_0 (requested)\n"
    );
}
#[test]
fn dry_run_is_not_silently_ignored_for_other_actions() {
    let f = Fixture::new(&[("app", "version 1")]);
    let result = f.run(&["-y", "download", "--manifest", "missing.json"]);
    assert!(!result.status.success());
}
#[test]
fn install_plan_discloses_unverified_archives() {
    let f = Fixture::new(&[("app", "version 1")]);
    let output = stdout(f.run(&["-y", "install", "app"]));
    assert!(output.contains("Archive availability and signatures are not checked"));
    assert!(output.contains("no packages will be installed"));
}
#[test]
fn deeply_nested_installation_graph_is_iterative() {
    let names: Vec<_> = (0..5000).map(|i| format!("pkg{i}")).collect();
    let fields: Vec<_> = (0..5000)
        .map(|i| {
            if i == 0 {
                "version 1".into()
            } else {
                format!("version 1 depends_run port:pkg{}", i - 1)
            }
        })
        .collect();
    let records: Vec<_> = names
        .iter()
        .zip(&fields)
        .map(|(name, field)| (name.as_str(), field.as_str()))
        .collect();
    let f = Fixture::new(&records);
    let output = stdout(f.run(&["-qy", "install", "pkg4999"]));
    assert_eq!(output.lines().count(), 5000);
    assert!(output.starts_with("install  pkg0"));
    assert!(output.ends_with("install  pkg4999 @1_0 (requested)\n"));
}

#[test]
fn image_matching_checks_epoch_as_well_as_version() {
    let f = Fixture::new(&[("app", "version 1 epoch 2")]);
    let db = Connection::open(f.dir.path().join("registry.db")).unwrap();
    db.execute_batch("ALTER TABLE ports ADD COLUMN epoch INTEGER DEFAULT 0; INSERT INTO ports VALUES(1,'app','1',0,'','imaged',1)").unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @1_0 (requested)\n"
    );
    db.execute("UPDATE ports SET epoch=2", []).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "activate app @1_0 (requested)\n"
    );
    db.execute_batch("UPDATE ports SET state='installed',epoch=1")
        .unwrap();
    let result = f.run(&["-y", "install", "app"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}
