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
        Connection::open(dir.path().join("registry.db")).unwrap().execute_batch("CREATE TABLE ports(id INTEGER PRIMARY KEY,name TEXT,version TEXT,revision INTEGER,variants TEXT,state TEXT); CREATE TABLE files(id INTEGER,path TEXT,actual_path TEXT,active INTEGER);").unwrap();
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
            ("app", "version 1 depends_run unsupported:base:base"),
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

#[test]
fn official_style_curl_index_plans_missing_defaults_and_provider_fallbacks() {
    let f = Fixture::new(&[
        (
            "curl",
            "version 8.23.0 revision 0 variants {ares ssl universal} depends_lib {port:zlib path:lib/libssl.dylib:openssl path:share/curl/ca-bundle.crt:curl-ca-bundle}",
        ),
        ("zlib", "version 1.3"),
        ("openssl", "version 3"),
        ("curl-ca-bundle", "version 8.23.0"),
    ]);
    let before = std::fs::read(f.dir.path().join("registry.db")).unwrap();
    let output = stdout(f.run(&["-qy", "install", "curl"]));
    assert!(output.starts_with("install  zlib @1.3_0 (dependency)\n"));
    assert!(output.contains("install  openssl @3_0 (dependency)"));
    assert!(output.contains("resolve  curl @8.23.0_0 (requested) [default variants unresolved]"));
    assert!(output.contains("provider checks pending: path:lib/libssl.dylib:openssl"));
    assert_eq!(
        std::fs::read(f.dir.path().join("registry.db")).unwrap(),
        before
    );
}
#[test]
fn incomplete_variant_metadata_does_not_approve_matching_images() {
    let f = Fixture::new(&[("app", "version 1 variants {ssl universal}")]);
    Connection::open(f.dir.path().join("registry.db"))
        .unwrap()
        .execute_batch("INSERT INTO ports VALUES(1,'app','1',0,'','installed')")
        .unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "resolve  app @1_0 (requested) [default variants unresolved]\n"
    );
    let output = f.run(&["-y", "install", "app", "+ssl"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot validate requested variant"));
}

fn portfile(f: &Fixture, name: &str, body: &str) {
    let path = f.dir.path().join("net").join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("Portfile"),
        format!("name {name}; version 2; {body}"),
    )
    .unwrap();
}

#[test]
fn local_install_preview_uses_evaluated_variants_and_runtime_dependencies() {
    let f = Fixture::new(&[
        (
            "app",
            "version 1 portdir net/app variants stale depends_run port:missing",
        ),
        ("base", "version 1 portdir net/base"),
        ("secure", "version 1 portdir net/secure"),
    ]);
    portfile(
        &f,
        "app",
        "depends_lib port:base; depends_build port:missing; variant tls {depends_run-append port:secure}; pre-activate {exec must-not-run}",
    );
    portfile(&f, "base", "");
    portfile(&f, "secure", "");
    let before = std::fs::read(f.dir.path().join("registry.db")).unwrap();
    let output = stdout(f.run(&["-q", "-y", "install", "app@2", "+tls"]));
    assert_eq!(
        output,
        "install  base @2_0 (dependency)\ninstall  secure @2_0 (dependency)\ninstall  app @2_0+tls (requested)\n"
    );
    assert_eq!(
        before,
        std::fs::read(f.dir.path().join("registry.db")).unwrap()
    );
    let output = stdout(f.run(&["-y", "install", "app"]));
    assert!(output.contains("evaluated local Portfiles"));
    assert!(output.contains("hooks are not executed"));
    assert!(!output.contains("secure @"));
}

#[test]
fn local_install_preview_propagates_explicit_inputs_to_dependencies() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
        ("extra", "version 1 portdir net/extra"),
    ]);
    portfile(&f, "app", "depends_lib port:base; variant tls {}");
    portfile(&f, "base", "variant tls {depends_run port:extra}");
    portfile(&f, "extra", "");
    assert_eq!(
        stdout(f.run(&["-q", "-y", "install", "app", "+tls"])),
        "install  extra @2_0 (dependency)\ninstall  base @2_0+tls (dependency)\ninstall  app @2_0+tls (requested)\n"
    );
}

#[test]
fn local_matching_active_images_skip_their_source_dependency_closure() {
    let f = Fixture::new(&[("app", "version 1 portdir net/app")]);
    portfile(
        &f,
        "app",
        "depends_run port:missing; variant tls {}; default_variants +tls",
    );
    Connection::open(f.dir.path().join("registry.db"))
        .unwrap()
        .execute(
            "INSERT INTO ports VALUES(1,'app','2',0,'+tls','installed')",
            [],
        )
        .unwrap();
    assert_eq!(
        stdout(f.run(&["-q", "-y", "install", "app"])),
        "active   app @2_0+tls (requested)\n"
    );
}

#[test]
fn local_install_preview_never_mixes_failed_sources_with_index_metadata() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
    ]);
    portfile(&f, "app", "depends_run port:base");
    for body in [
        None,
        Some("unsupported_command"),
        Some("depends_run port:app"),
    ] {
        if let Some(body) = body {
            portfile(&f, "base", body);
        }
        let output = f.run(&["-y", "install", "app"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn shared_dependencies_reject_incompatible_evaluations_but_allow_equal_results() {
    let f = Fixture::new(&[
        ("left", "version 1 portdir net/left"),
        ("right", "version 1 portdir net/right"),
        ("shared", "version 1 portdir net/shared"),
    ]);
    for name in ["left", "right"] {
        portfile(&f, name, "depends_run port:shared; variant tls {}");
    }
    portfile(&f, "shared", "variant tls {}");
    let output = f.run(&["-y", "install", "left", "+tls", "right", "-tls"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicting evaluated build inputs"));
    portfile(&f, "shared", "");
    let output = stdout(f.run(&["-q", "-y", "install", "left", "+tls", "right", "-tls"]));
    assert_eq!(output.matches("shared @").count(), 1);
}

#[test]
fn explicit_dependency_roots_override_inherited_variant_choices() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
    ]);
    portfile(&f, "app", "depends_run port:base; variant tls {}");
    portfile(&f, "base", "variant tls {}");
    let output = stdout(f.run(&["-q", "-y", "install", "app", "+tls", "base", "-tls"]));
    assert_eq!(
        output,
        "install  base @2_0 (requested)\ninstall  app @2_0+tls (requested)\n"
    );
}

#[test]
fn equal_shared_metadata_still_propagates_each_context_to_descendants() {
    let f = Fixture::new(&[
        ("left", "version 1 portdir net/left"),
        ("right", "version 1 portdir net/right"),
        ("shared", "version 1 portdir net/shared"),
        ("leaf", "version 1 portdir net/leaf"),
    ]);
    for name in ["left", "right"] {
        portfile(&f, name, "depends_run port:shared");
    }
    portfile(&f, "shared", "depends_run port:leaf");
    portfile(&f, "leaf", "variant tls {}");
    let output = f.run(&["-y", "install", "left", "+tls", "right", "-tls"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("leaf: conflicting evaluated build inputs")
    );
}

#[test]
fn local_install_options_reach_dependencies_and_version_checks_use_source() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
    ]);
    portfile(&f, "app", "depends_run port:base");
    portfile(&f, "base", "if {[option build.jobs] == 7} {revision 3}");
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app@2", "build.jobs=7"])),
        "install  base @2_3 (dependency)\ninstall  app @2_0 (requested)\n"
    );
    let output = f.run(&["-qy", "install", "app@1"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn real_curl_portfile_resolves_missing_index_defaults_in_install_preview() {
    // Curl and its PortGroups are pinned upstream sources. Dependency Portfiles
    // are deliberately minimal: this tests planning, not building those ports.
    let f = Fixture::new(&[
        (
            "curl",
            "version 8.23.0 portdir net/curl variants {ares ssl universal}",
        ),
        ("zlib", "version 1 portdir net/zlib"),
        ("brotli", "version 1 portdir net/brotli"),
        ("nghttp2", "version 1 portdir net/nghttp2"),
        ("libidn2", "version 1 portdir net/libidn2"),
        ("libpsl", "version 1 portdir net/libpsl"),
        ("openssl", "version 1 portdir net/openssl"),
        ("curl-ca-bundle", "version 1 portdir net/curl-ca-bundle"),
        ("zstd", "version 1 portdir net/zstd"),
        ("c-ares", "version 1 portdir net/c-ares"),
    ]);
    for name in [
        "curl",
        "zlib",
        "brotli",
        "nghttp2",
        "libidn2",
        "libpsl",
        "openssl",
        "curl-ca-bundle",
        "zstd",
        "c-ares",
    ] {
        portfile(&f, name, "");
    }
    std::fs::write(
        f.dir.path().join("net/curl/Portfile"),
        include_str!("fixtures/curl/Portfile"),
    )
    .unwrap();
    let groups = f.dir.path().join("_resources/port1.0/group");
    std::fs::create_dir_all(&groups).unwrap();
    for (name, source) in [
        (
            "clang_dependency-1.0.tcl",
            include_str!("fixtures/curl/clang_dependency-1.0.tcl"),
        ),
        (
            "muniversal-1.0.tcl",
            include_str!("fixtures/curl/muniversal-1.0.tcl"),
        ),
    ] {
        std::fs::write(groups.join(name), source).unwrap();
    }
    let output = stdout(f.run(&["-qy", "install", "curl"]));
    assert!(output.contains("curl @8.23.0_0+brotli+http2+idn+psl+ssl+zstd (requested)"));
    assert!(!output.contains("unresolved"));
    assert!(!output.contains("c-ares @"));
    let output = stdout(f.run(&["-qy", "install", "curl", "+ares"]));
    assert!(output.contains("install  c-ares @2_0 (dependency)"));
    assert!(output.contains("curl @8.23.0_0+ares+brotli+http2+idn+psl+ssl+zstd (requested)"));
}

#[test]
fn local_install_preview_reads_user_configuration_and_filtered_environment() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
    ]);
    let home = f.dir.path().join("home");
    std::fs::create_dir_all(home.join(".macports")).unwrap();
    std::fs::write(home.join(".macports/macports.conf"), "prefix ~/prefix\nframeworks_dir ~/prefix/Library/Frameworks\nbinpath /configured/bin\nextra_env NO_COLOR\nproxy_http http://configured.invalid:8080\nproxy_override_env yes\n").unwrap();
    portfile(
        &f,
        "app",
        r#"
        if {$env(PATH) ne "/configured/bin"} {error "wrong PATH"}
        if {$env(LANG) ne "en_US.UTF-8"} {error "wrong LANG"}
        if {$env(NO_COLOR) ne "1"} {error "extra_env missing"}
        if {[info exists env(PORT_RS_INDEX)]} {error "host environment leaked"}
        if {$env(http_proxy) ne "http://configured.invalid:8080"} {error "wrong proxy"}
        if {[file tail $prefix] ne "prefix" || [file tail [file dirname $prefix]] ne "home"} {error "wrong prefix"}
        if {$prefix_frozen ne $prefix} {error "wrong frozen prefix"}
        if {$frameworks_dir ne "$prefix/Library/Frameworks"} {error "wrong frameworks"}
        depends_run port:base
    "#,
    );
    portfile(
        &f,
        "base",
        r#"if {$env(PATH) ne "/configured/bin"} {error "dependency environment missing"}"#,
    );
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  base @2_0 (dependency)\ninstall  app @2_0 (requested)\n"
    );
    assert!(!home.join("prefix").exists());
}

#[test]
fn local_install_applies_configured_variants_to_roots_and_dependencies() {
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("base", "version 1 portdir net/base"),
    ]);
    let home = f.dir.path().join("home");
    std::fs::create_dir_all(home.join(".macports")).unwrap();
    std::fs::write(
        home.join(".macports/macports.conf"),
        "variants_conf ~/.macports/variants.conf\n",
    )
    .unwrap();
    std::fs::write(home.join(".macports/variants.conf"), "+ssl -docs\n-ssl\n").unwrap();
    portfile(
        &f,
        "app",
        "variant ssl {}; variant docs {}; default_variants +docs; depends_run port:base",
    );
    portfile(
        &f,
        "base",
        "variant ssl {}; variant docs {}; default_variants +docs",
    );
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  base @2_0+ssl (dependency)\ninstall  app @2_0+ssl (requested)\n"
    );
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app", "-ssl", "base", "+docs"])),
        "install  base @2_0+docs+ssl (requested)\ninstall  app @2_0 (requested)\n"
    );
}

#[test]
fn local_preview_skips_available_providers_without_loading_fallback_portfiles() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(&[
        ("app", "version 1 portdir net/app"),
        ("fallback", "version 1 portdir net/fallback"),
    ]);
    let home = f.dir.path().join("home");
    std::fs::create_dir_all(home.join(".macports")).unwrap();
    std::fs::create_dir_all(home.join("prefix/bin")).unwrap();
    std::fs::write(home.join(".macports/macports.conf"), "prefix ~/prefix\n").unwrap();
    std::fs::write(home.join("prefix/provided"), b"present").unwrap();
    let tool = home.join("prefix/bin/fixture-tool");
    std::fs::write(&tool, b"never executed").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    portfile(
        &f,
        "app",
        "set env(PATH) [file join $prefix bin]; depends_run path:provided:fallback bin:fixture-tool:absent-from-index",
    );
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @2_0 (requested)\n"
    );
    // An explicit request still requires its source even if a provider exists.
    let output = f.run(&["-qy", "install", "app", "fallback"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    portfile(&f, "fallback", "");
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app", "fallback"])),
        "install  fallback @2_0 (requested)\ninstall  app @2_0 (requested)\n"
    );
    std::fs::remove_file(home.join("prefix/provided")).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  fallback @2_0 (dependency)\ninstall  app @2_0 (requested)\n"
    );
}

#[test]
fn local_preview_accepts_active_dependencies_without_fallback_index_metadata() {
    let f = Fixture::new(&[("app", "version 1 portdir net/app")]);
    portfile(
        &f,
        "app",
        "depends_run port:already-installed bin:missing-tool:active-fallback",
    );
    let db = Connection::open(f.dir.path().join("registry.db")).unwrap();
    db.execute_batch("INSERT INTO ports VALUES(1,'already-installed','9',0,'','installed'); INSERT INTO ports VALUES(2,'active-fallback','8',0,'','installed');").unwrap();
    let before = std::fs::read(f.dir.path().join("registry.db")).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @2_0 (requested)\n"
    );
    assert_eq!(
        std::fs::read(f.dir.path().join("registry.db")).unwrap(),
        before
    );
    db.execute("UPDATE ports SET state='imaged' WHERE id=2", [])
        .unwrap();
    let output = f.run(&["-qy", "install", "app"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn local_preview_uses_configured_registry_and_preserves_explicit_overrides() {
    let f = Fixture::new(&[("app", "version 1 portdir net/app")]);
    portfile(&f, "app", "");
    let home = f.dir.path().join("home");
    std::fs::create_dir_all(home.join(".macports")).unwrap();
    std::fs::write(
        home.join(".macports/macports.conf"),
        "portdbpath ~/database\n",
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(["-qy", "install", "app"])
            .env("PORT_RS_INDEX", f.dir.path().join("PortIndex"))
            .env_remove("PORT_RS_REGISTRY")
            .env("HOME", &home)
            .env("PATH", "")
            .env("NO_COLOR", "1")
            .current_dir(f.dir.path())
            .output()
            .unwrap()
    };
    // A dry run against a missing configured registry never creates its parent.
    assert_eq!(stdout(run()), "install  app @2_0 (requested)\n");
    assert!(!home.join("database").exists());
    std::fs::create_dir_all(home.join("database/registry")).unwrap();
    let database = home.join("database/registry/registry.db");
    std::fs::copy(f.dir.path().join("registry.db"), &database).unwrap();
    let db = Connection::open(&database).unwrap();
    db.execute_batch("INSERT INTO ports VALUES(1,'app','2',0,'','installed');")
        .unwrap();
    drop(db);
    let before = std::fs::read(&database).unwrap();
    assert_eq!(stdout(run()), "active   app @2_0 (requested)\n");
    assert_eq!(std::fs::read(&database).unwrap(), before);
    // Fixture.run selects the original empty registry explicitly.
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @2_0 (requested)\n"
    );
    std::fs::write(&database, b"not a database").unwrap();
    let output = run();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(&database).unwrap(), b"not a database");
}

#[test]
fn local_install_resolves_configured_archive_defaults_before_portfile_mutations() {
    let f = Fixture::new(&[("app", "version 1 portdir net/app")]);
    let resources = f.dir.path().join("_resources/custom");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("mirrors.tcl"),
        include_str!("fixtures/archive-sites/archive_sites.tcl"),
    )
    .unwrap();
    let home = f.dir.path().join("home/.macports");
    std::fs::create_dir_all(&home).unwrap();
    let sites = f.dir.path().join("sites.conf");
    std::fs::write(&sites, "name configured\nurls https://custom/\n").unwrap();
    std::fs::write(
        home.join("macports.conf"),
        format!(
            "archive_sites_conf {}\ncxx_stdlib libc++\ndelete_la_files yes\n",
            sites.display()
        ),
    )
    .unwrap();
    portfile(
        &f,
        "app",
        r#"
        archive_sites.listpath custom
        archive_sites.listfile mirrors.tcl
        archive_sites-append extra::tbz2
        if {${archive_sites} ne "macports_archives::tbz2 configured::tbz2 extra::tbz2"} {error "incorrect archive defaults: ${archive_sites}"}
        revision 5
    "#,
    );
    let before = std::fs::read(f.dir.path().join("registry.db")).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @2_5 (requested)\n"
    );
    assert_eq!(
        std::fs::read(f.dir.path().join("registry.db")).unwrap(),
        before
    );
    // Explicit values avoid evaluating a broken resource at the default location.
    let default = f.dir.path().join("_resources/port1.0/fetch");
    std::fs::create_dir_all(&default).unwrap();
    std::fs::write(default.join("archive_sites.tcl"), "error broken-resource").unwrap();
    portfile(&f, "app", "archive_sites explicit::tbz2");
    assert!(f.run(&["-qy", "install", "app"]).status.success());
    portfile(&f, "app", "archive_sites-append extra::tbz2");
    let output = f.run(&["-qy", "install", "app"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("broken-resource"));
    assert_eq!(
        std::fs::read(f.dir.path().join("registry.db")).unwrap(),
        before
    );
}

#[test]
fn local_preview_binds_selected_registry_before_portfile_evaluation() {
    let f = Fixture::new(&[("app", "version 1 portdir net/app")]);
    let database = f.dir.path().join("registry.db");
    Connection::open(&database)
        .unwrap()
        .execute_batch("INSERT INTO ports VALUES(1,'base','3',2,'+ssl','imaged');")
        .unwrap();
    portfile(
        &f,
        "app",
        r#"
        if {![registry_exists base 3 2 +ssl]} {error missing-registry-binding}
        if {![registry_exists_for_name base]} {error missing-name-binding}
        if {[registry_exists base 3]} {error wrong-defaults}
        revision 6
    "#,
    );
    let before = std::fs::read(&database).unwrap();
    assert_eq!(
        stdout(f.run(&["-qy", "install", "app"])),
        "install  app @2_6 (requested)\n"
    );
    assert_eq!(std::fs::read(&database).unwrap(), before);
}
