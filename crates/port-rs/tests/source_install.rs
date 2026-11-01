//! Offline end-to-end source installs through the CLI, with Portfiles that
//! need no downloads, a private prefix and a private registry.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// `_resources/port1.0/livecheck/master-sites.tcl` from the ports tree.
const MASTER_SITES: &str = r#"set livecheck.type "regex"
if {${livecheck.name} eq "default"} {
    set livecheck.name ${name}
}
if {${livecheck.distname} eq "default"} {
    set livecheck.distname ${livecheck.name}
}
if {!$has_homepage || ${livecheck.url} eq ${homepage}} {
    if {!$has_master_sites || [llength ${master_sites}] == 0} {
        set livecheck.type "none"
    } else {
        set livecheck.url [lindex ${master_sites} 0]
    }
}
if {${livecheck.regex} eq ""} {
    set livecheck.regex [list "[quotemeta ${livecheck.distname}]-(\\d+(?:\\.\\d+)*)"]
}
"#;

struct Tree {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn portfile(name: &str, version: &str, depends: &str) -> String {
    format!(
        r#"PortSystem 1.0
name {name}
version {version}
categories devel
platforms darwin
license MIT
maintainers nomaintainer
description {name}
long_description {name} for tests
homepage https://example.org
distfiles
supported_archs noarch
use_configure no
{depends}
notes "{name} is ready"
build {{}}
pre-destroot {{ ui_debug "staging {name}" }}
destroot {{
    xinstall -d ${{destroot}}${{prefix}}/share/{name}
    system "echo {name}-{version} > ${{destroot}}${{prefix}}/share/{name}/version"
}}
"#
    )
}

impl Tree {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir_all(root.join("ports/_resources/port1.0/group")).unwrap();
        fs::write(
            root.join("macports.conf"),
            format!(
                "prefix {0}/prefix\nportdbpath {0}/db\napplications_dir {0}/prefix/Applications\nframeworks_dir {0}/prefix/Library/Frameworks\nplace_worksymlink no\nsources_conf {0}/sources.conf\n",
                root.display()
            ),
        )
        .unwrap();
        fs::write(root.join("sources.conf"), "").unwrap();
        let livecheck = root.join("ports/_resources/port1.0/livecheck");
        fs::create_dir_all(&livecheck).unwrap();
        fs::write(
            livecheck.join("fallback.tcl"),
            "source [getdefaultportresourcepath \"port1.0/livecheck\"]/master-sites.tcl\n",
        )
        .unwrap();
        fs::write(livecheck.join("master-sites.tcl"), MASTER_SITES).unwrap();
        let tree = Self { _temp: temp, root };
        tree.port("lib", "1", "");
        tree.port("app", "1", "depends_lib port:lib");
        tree
    }

    fn port(&self, name: &str, version: &str, depends: &str) {
        let directory = self.root.join("ports/devel").join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Portfile"), portfile(name, version, depends)).unwrap();
        let mut index = String::new();
        let mut ports: Vec<String> = fs::read_dir(self.root.join("ports/devel"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        ports.sort();
        for port in &ports {
            let port = port.as_str();
            let Ok(text) =
                fs::read_to_string(self.root.join("ports/devel").join(port).join("Portfile"))
            else {
                continue;
            };
            let version = text
                .lines()
                .find_map(|line| line.strip_prefix("version "))
                .unwrap()
                .trim();
            let depends: String = text
                .lines()
                .filter(|line| line.starts_with("depends_"))
                .filter_map(|line| line.split_once(' '))
                .map(|(field, value)| format!(" {field} {{{value}}}"))
                .collect();
            let body =
                format!("name {port} portdir devel/{port} version {version} revision 0{depends}");
            index.push_str(&format!("{port} {}\n{body}\n", body.len() + 1));
        }
        fs::write(self.root.join("ports/PortIndex"), index).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .arg("--index-file")
            .arg(self.root.join("ports/PortIndex"))
            .args(args)
            .env("PORTSRC", self.root.join("macports.conf"))
            .env("HOME", &self.root)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn file(&self, path: &str) -> PathBuf {
        self.root.join("prefix").join(path)
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn installs_dependencies_first_then_upgrades_and_removes_old_images() {
    let tree = Tree::new();
    let installed = tree.ok(&["install", "app"]);
    assert!(
        installed.contains("app has the following notes"),
        "{installed}"
    );
    assert_eq!(read(&tree.file("share/lib/version")), "lib-1\n");
    assert_eq!(read(&tree.file("share/app/version")), "app-1\n");
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("app @1_0 (active)") && list.contains("lib @1_0 (active)"),
        "{list}"
    );
    assert!(tree.ok(&["install", "app"]).contains("already installed"));

    tree.port("app", "2", "depends_lib port:lib");
    assert!(tree.ok(&["outdated"]).contains("app"));
    tree.ok(&["upgrade", "app"]);
    assert_eq!(read(&tree.file("share/app/version")), "app-2\n");
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("app @2_0 (active)") && list.contains("app @1_0\n"),
        "{list}"
    );
    assert!(
        tree.ok(&["outdated"])
            .contains("No installed ports are outdated")
    );

    tree.port("app", "3", "depends_lib port:lib");
    tree.ok(&["upgrade", "-u", "app"]);
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("app @3_0 (active)") && !list.contains("app @2_0"),
        "{list}"
    );

    let blocked = tree.run(&["uninstall", "lib"]);
    assert!(!blocked.status.success(), "lib still has a dependent");
    let ambiguous = tree.run(&["uninstall", "app"]);
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("specify one"));
    tree.ok(&["uninstall", "app", "@1_0"]);
    tree.ok(&["uninstall", "app"]);
    tree.ok(&["uninstall", "lib"]);
    assert!(!tree.file("share/lib/version").exists());
}

#[test]
fn phase_logs_are_filtered_by_phase_and_level_and_dropped_after_install() {
    let tree = Tree::new();
    tree.ok(&["destroot", "lib"]);
    let log = tree.ok(&["log", "lib"]);
    assert!(log.contains("--->  Staging lib"), "{log}");
    assert!(log.contains("DEBUG: staging lib"), "{log}");
    let destroot = tree.ok(&["log", "--phase", "destroot", "lib"]);
    assert!(
        destroot.contains("DEBUG: staging lib") && !destroot.contains("Extracting"),
        "{destroot}"
    );
    let messages = tree.ok(&["log", "--level", "msg", "lib"]);
    assert!(
        messages.contains("Staging lib") && !messages.contains("DEBUG"),
        "{messages}"
    );
    assert!(
        tree.ok(&["logfile", "lib"])
            .trim_end()
            .ends_with("main.log")
    );
    tree.ok(&["install", "lib"]);
    let missing = tree.run(&["log", "lib"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Log file for port lib not found"));
}

#[test]
fn distfiles_lists_paths_checksums_and_urls_in_site_order() {
    let tree = Tree::new();
    let text = portfile("dist", "1", "")
        .replace(
            "distfiles\n",
            "master_sites https://example.org/files/ https://mirror.example/dist/\ndistfiles dist-1.tar.gz\nchecksums sha256 abc size 3\n",
        );
    let directory = tree.root.join("ports/devel/dist");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
    let output = tree.ok(&["distfiles", "dist", "lib"]);
    let path = tree.root.join("db/distfiles/dist/dist-1.tar.gz");
    assert!(
        output.starts_with(&format!(
            "--->  Distfiles for dist\n[dist-1.tar.gz] {}\n sha256: abc\n size: 3\n  https://example.org/files/dist-1.tar.gz\n  https://mirror.example/dist/dist-1.tar.gz\n",
            path.display()
        )),
        "{output}"
    );
    assert!(output.ends_with("--->  Distfiles for lib\n"), "{output}");
}

#[test]
fn load_requires_an_installed_plist_and_ignores_ports_without_startup_items() {
    let tree = Tree::new();
    let text = portfile("svc", "1", "").replace(
        "build {}",
        "startupitem.create yes\nstartupitem.location LaunchAgents\nstartupitem.executable /usr/bin/true\nbuild {}",
    );
    let directory = tree.root.join("ports/devel/svc");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
    tree.ok(&["load", "lib"]);
    let output = tree.run(&["load", "svc"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Launchd plist /Library/LaunchAgents/org.macports.svc.plist was not found"),
        "{error}"
    );
}

#[test]
fn archive_stores_inactive_images_that_install_later_activates_without_rebuilding() {
    let tree = Tree::new();
    tree.ok(&["archive", "app"]);
    let list = tree.ok(&["installed"]);
    assert!(list.contains("lib @1_0 (active)"), "{list}");
    assert!(
        list.contains("app @1_0\n") && !list.contains("app @1_0 (active)"),
        "{list}"
    );
    assert!(!tree.file("share/app/version").exists());
    let again = tree.run(&["archive", "app"]);
    assert!(String::from_utf8_lossy(&again.stderr).contains("app @1_0 is already archived"));

    let installed = tree.run(&["install", "app"]);
    assert!(installed.status.success());
    let progress = String::from_utf8_lossy(&installed.stderr);
    assert!(
        progress.contains("Activating app @1_0") && !progress.contains("Building app"),
        "{progress}"
    );
    assert_eq!(read(&tree.file("share/app/version")), "app-1\n");
    assert!(tree.ok(&["installed"]).contains("app @1_0 (active)"));
    assert!(tree.ok(&["installed", "requested"]).contains("app @1_0"));
}

#[test]
fn reclaim_removes_inactive_and_unneeded_ports_and_unused_files() {
    let tree = Tree::new();
    tree.ok(&["install", "app"]);
    tree.port("app", "2", "depends_lib port:lib");
    tree.ok(&["upgrade", "app"]);
    let unused = tree.root.join("db/distfiles/old/old-1.tar.gz");
    fs::create_dir_all(unused.parent().unwrap()).unwrap();
    fs::write(&unused, "x").unwrap();
    fs::create_dir_all(tree.root.join("db/build/leftover")).unwrap();

    tree.ok(&["-y", "reclaim"]);
    assert!(unused.exists() && tree.root.join("db/build/leftover").exists());
    assert!(tree.ok(&["installed"]).contains("app @1_0"));

    let output = tree.run(&["reclaim"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let progress = String::from_utf8_lossy(&output.stderr);
    assert!(
        progress.contains("Found no unrequested ports without requested dependents."),
        "{progress}"
    );
    assert!(progress.contains("Uninstalling app @1_0"), "{progress}");
    assert!(!unused.exists() && !unused.parent().unwrap().exists());
    assert!(!tree.root.join("db/build/leftover").exists());
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("app @2_0 (active)")
            && list.contains("lib @1_0 (active)")
            && !list.contains("app @1_0"),
        "{list}"
    );
    assert!(
        read(&tree.root.join("db/last_reclaim"))
            .trim()
            .parse::<u64>()
            .is_ok()
    );

    tree.ok(&["snapshot"]);
    tree.ok(&["snapshot", "--note", "recent"]);
    age_first_snapshot(&tree.root.join("db/registry/registry.db"));
    tree.ok(&["setunrequested", "app"]);
    let output = tree.run(&["reclaim"]);
    let progress = String::from_utf8_lossy(&output.stderr);
    let app = progress.find("Uninstalling app @2_0").expect(&progress);
    let lib = progress.find("Uninstalling lib @1_0").expect(&progress);
    assert!(app < lib, "{progress}");
    assert!(tree.ok(&["installed"]).contains("No ports are installed"));

    let list = tree.ok(&["snapshot", "--list"]);
    assert!(
        list.contains("recent") && !list.contains("snapshot created for migration"),
        "{list}"
    );
    tree.ok(&["reclaim", "--disable-reminders"]);
    assert_eq!(read(&tree.root.join("db/last_reclaim")), "disabled\n");
}

#[test]
fn rev_upgrade_reports_and_retries_ports_with_missing_libraries() {
    let tree = Tree::new();
    let text = portfile("linked", "1", "").replace(
        "system \"echo linked-1",
        "xinstall -d ${destroot}${prefix}/bin\n    copy /bin/echo ${destroot}${prefix}/bin/echoer\n    system \"/usr/bin/install_name_tool -change /usr/lib/libSystem.B.dylib /no/z.dylib ${destroot}${prefix}/bin/echoer 2>/dev/null\"\n    system \"echo linked-1",
    );
    let directory = tree.root.join("ports/devel/linked");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
    tree.ok(&["install", "--no-rev-upgrade", "lib", "linked"]);
    let clean = tree.run(&["rev-upgrade"]);
    let progress = String::from_utf8_lossy(&clean.stderr);
    assert!(progress.contains("Found 1 broken file"), "{progress}");

    let report = tree.ok(&["-y", "rev-upgrade"]);
    assert_eq!(
        report,
        format!(
            "     linked @1_0\n         {}\n",
            tree.file("bin/echoer").display()
        )
    );
    assert!(!clean.status.success());
    assert!(
        String::from_utf8_lossy(&clean.stderr)
            .contains("Port linked still broken after rebuilding 3 times"),
        "{progress}"
    );
    assert_eq!(
        progress.matches("Uninstalling linked @1_0").count(),
        3,
        "{progress}"
    );
    assert!(tree.ok(&["installed"]).contains("linked @1_0 (active)"));
}

#[test]
fn failing_phases_name_the_target_and_point_to_the_log() {
    let tree = Tree::new();
    let text = portfile("broken", "1", "").replace("build {}", "build { system \"exit 3\" }");
    let directory = tree.root.join("ports/devel/broken");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
    let output = tree.run(&["install", "broken"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Error: Failed to build broken: command execution failed"),
        "{error}"
    );
    assert!(error.contains("main.log for details."), "{error}");
    let log = tree.ok(&["log", "--phase", "build", "broken"]);
    assert!(
        log.contains("DEBUG: Command failed: exit 3") && log.contains("DEBUG: Exit code: 3"),
        "{log}"
    );
    assert!(log.contains("Error: Failed to build broken"), "{log}");
}

#[test]
fn snapshots_record_compare_export_and_restore_installed_ports() {
    let tree = Tree::new();
    assert!(
        tree.ok(&["snapshot", "--list"])
            .starts_with("There are no snapshots.")
    );
    tree.ok(&["install", "app"]);
    tree.ok(&["snapshot", "--note", "before", "cleanup"]);
    let list = tree.ok(&["snapshot", "--list"]);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(
        lines[0].split_whitespace().collect::<Vec<_>>(),
        ["ID", "Created", "Note"]
    );
    assert!(
        lines[2].starts_with(" 1  ") && lines[2].ends_with("before cleanup"),
        "{list}"
    );
    assert!(
        tree.ok(&["snapshot", "--diff", "1"])
            .ends_with("The current state and the specified snapshot match.\n")
    );

    let exported = tree.ok(&["snapshot", "--export", "1"]);
    let json: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(json["metadata"]["type"], "org.macports/snapshot/v1");
    assert_eq!(json["metadata"]["note"], "before cleanup");
    let ports = json["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 2);
    assert_eq!(ports[0]["port_name"], "app");
    assert_eq!(ports[0]["requested"], 1);
    assert!(
        ports[1]["port_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f.as_str().unwrap().ends_with("share/lib/version"))
    );

    tree.ok(&["uninstall", "app"]);
    tree.ok(&["uninstall", "lib"]);
    let diff = tree.ok(&["snapshot", "--diff", "1", "--all"]);
    assert!(
        diff.contains(
            "The following ports are in the snapshot but not installed:\n - app \n - lib \n"
        ),
        "{diff}"
    );

    let file = tree.root.join("snapshot.json");
    fs::write(&file, &exported).unwrap();
    assert!(
        tree.ok(&["snapshot", "--import", file.to_str().unwrap()])
            .starts_with("Snapshot successfully imported with ID 2.")
    );
    tree.ok(&["snapshot", "--delete", "1"]);
    assert!(!tree.run(&["snapshot", "--delete", "1"]).status.success());
    let multiple = tree.run(&["snapshot", "--list", "--create"]);
    assert!(String::from_utf8_lossy(&multiple.stderr).contains("Only one of the --list"));

    let restored = tree.run(&["restore", "--last"]);
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let progress = String::from_utf8_lossy(&restored.stderr);
    assert!(
        progress.contains("===>  Restoring port 1 of 1: app")
            && progress.contains("Migration finished.\n"),
        "{progress}"
    );
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("app @1_0 (active)") && list.contains("lib @1_0 (active)"),
        "{list}"
    );
    let requested = tree.ok(&["installed", "requested"]);
    assert!(
        requested.contains("app @1_0") && !requested.contains("lib @1_0"),
        "{requested}"
    );

    let migrated = tree.run(&["migrate"]);
    assert!(
        migrated.status.success(),
        "{}",
        String::from_utf8_lossy(&migrated.stderr)
    );
    let progress = String::from_utf8_lossy(&migrated.stderr);
    assert!(
        progress.contains("Done: Snapshot '3'") && progress.contains("Restoring ports..."),
        "{progress}"
    );
    assert!(tree.ok(&["installed"]).contains("app @1_0 (active)"));
}

/// Age the first snapshot past reclaim's 30-day limit.
fn age_first_snapshot(database: &Path) {
    port_rs::registry::sqlite::Connection::open(database)
        .unwrap()
        .execute(
            "UPDATE snapshots SET created_at='2000-01-01 00:00:00' WHERE id=1",
            (),
        )
        .unwrap();
}

#[test]
fn lint_reports_errors_and_warnings_like_portlint() {
    let tree = Tree::new();
    let clean = tree.run(&["lint", "lib"]);
    let progress = String::from_utf8_lossy(&clean.stderr);
    assert!(
        progress.contains("--->  Verifying Portfile for lib"),
        "{progress}"
    );
    assert!(
        progress.contains("--->  0 errors and 1 warnings found."),
        "{progress}"
    );
    assert!(progress.contains("Warning: Unnecessary platforms line as darwin is the default"));

    let text = "PortSystem 1.0\nname flawed\nversion 1.0\ncategories other\nplatforms darwin\nlicense GPL3\nmaintainers someone@example.org\ndescription flawed\nlong_description flawed\nhomepage https://example.org\nmaster_sites https://example.org/\nchecksums md5 0123456789abcdef0123456789abcdef\ndepends_lib port:missing\nvariant extra {}\nconfigure {}\nbuild { system \"cp /opt/local/x y\" }   \n";
    let directory = tree.root.join("ports/devel/flawed");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
    let output = tree.run(&["lint", "--nitpick", "flawed"]);
    assert!(!output.status.success());
    let progress = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "Error: Line 16 hardcodes /opt/local, use ${prefix} instead",
        "Warning: Line 15 should say \"use_configure no\" instead of declaring an empty configure phase",
        "Warning: Line 16 has trailing whitespace before newline",
        "Warning: Line 2 should be a newline (after PortSystem)",
        "Error: invalid license 'GPL3': missing hyphen before version",
        "Warning: Maintainer email address should be obfuscated as example.org:someone",
        "Error: Unknown dependency: missing",
        "Warning: Variant extra does not have a description",
        "Warning: missing recommended checksum type: rmd160",
        "Warning: checksum type is insecure on its own: md5",
        "Error: Portfile parent directory devel does not match primary category other",
    ] {
        assert!(
            progress.contains(expected),
            "missing {expected:?} in\n{progress}"
        );
    }
    assert!(progress.contains("--->  4 errors and"), "{progress}");
}

/// Serve one fixed body to every request on a local port.
fn serve(body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://{address}/")
}

#[test]
fn livecheck_extracts_the_newest_version_with_the_port_regex() {
    let tree = Tree::new();
    let url = serve("<a href=app-1.5.tar.gz>x</a> <a href=app-2.0.tar.gz>y</a>\napp-1.10.tar.gz\n");
    let text = portfile("checked", "1.5", "").replace(
        "build {}",
        &format!(
            "livecheck.url {url}\nlivecheck.regex {{app-(\\d+(?:\\.\\d+)*)\\.tar}}\nbuild {{}}"
        ),
    );
    let directory = tree.root.join("ports/devel/checked");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    let newest = portfile("newest", "2.0", "").replace(
        "build {}",
        &format!(
            "livecheck.url {url}\nlivecheck.regex {{app-(\\d+(?:\\.\\d+)*)\\.tar}}\nbuild {{}}"
        ),
    );
    let directory = tree.root.join("ports/devel/newest");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), newest).unwrap();
    tree.port("lib", "1", "");
    assert_eq!(
        tree.ok(&["livecheck", "checked", "newest", "lib"]),
        "checked seems to have been updated (port version: 1.5, new version: 2.0)\n"
    );
}

#[test]
fn distcheck_sizes_distfiles_and_reports_unavailable_ones() {
    let tree = Tree::new();
    let url = serve("0123456789");
    for (name, sites, kind) in [
        ("sized", url.as_str(), "filesize"),
        ("gone", "http://127.0.0.1:9/", "moddate"),
    ] {
        let text = portfile(name, "1", "").replace(
            "distfiles\n",
            &format!("master_sites {sites}\ndistfiles {name}-1.tar.gz\ndistcheck.type {kind}\n"),
        );
        let directory = tree.root.join("ports/devel").join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Portfile"), text).unwrap();
    }
    tree.port("lib", "1", "");
    assert_eq!(tree.ok(&["distcheck", "sized"]), "sized: 10\n");
    let gone = tree.run(&["distcheck", "gone"]);
    assert!(
        String::from_utf8_lossy(&gone.stderr)
            .contains("Error: no mirror had gone-1.tar.gz for gone"),
        "{}",
        String::from_utf8_lossy(&gone.stderr)
    );
}

#[test]
fn mirror_fetches_verified_distfiles_and_records_them() {
    let tree = Tree::new();
    let url = serve("0123456789");
    let sha = "84d89877f0d4041efb6bf91a16f0248f2fd573e6af05c19f96bedb9f882f7882";
    for (name, sum, license) in [
        ("good", sha, "MIT"),
        (
            "bad",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "MIT",
        ),
        ("private", sha, "NoMirror"),
    ] {
        let text = portfile(name, "1", "")
            .replace(
                "distfiles\n",
                &format!("master_sites {url}\ndistfiles {name}-1.tar.gz\nchecksums sha256 {sum} size 10\n"),
            )
            .replace("license MIT", &format!("license {license}"));
        let directory = tree.root.join("ports/devel").join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Portfile"), text).unwrap();
    }
    tree.port("lib", "1", "");
    tree.ok(&["mirror", "good", "private"]);
    let good = tree.root.join("db/distfiles/good/good-1.tar.gz");
    assert_eq!(read(&good), "0123456789");
    assert!(!tree.root.join("db/distfiles/private").exists());
    let map = port_rs::filemap::Filemap::load(&tree.root.join("db/distfiles_mirror.db")).unwrap();
    assert_eq!(
        map.entries(),
        [(good.to_string_lossy().into_owned(), "1".to_string())]
    );
    let bad = tree.run(&["mirror", "bad"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("portmirror: checksum failed"));
    assert!(!tree.root.join("db/distfiles/bad/bad-1.tar.gz").exists());
    tree.ok(&["mirror", "--new", "private"]);
    let map = port_rs::filemap::Filemap::load(&tree.root.join("db/distfiles_mirror.db")).unwrap();
    assert!(map.entries().is_empty());
}

#[test]
fn bump_rewrites_stale_checksums_and_resets_the_revision() {
    let tree = Tree::new();
    let url = serve("0123456789");
    let stale = "1111111111111111111111111111111111111111111111111111111111111111";
    let text = portfile("bumped", "2", "").replace(
        "distfiles\n",
        &format!("revision 3\nmaster_sites {url}\ndistfiles bumped-2.tar.gz\nchecksums sha256 {stale} \\\n          size 10\n"),
    );
    let directory = tree.root.join("ports/devel/bumped");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), &text).unwrap();
    tree.port("lib", "1", "");

    let patched = tree.ok(&["bump", "--patch", "bumped"]);
    assert!(
        patched.contains("Portfile.patch successfully created"),
        "{patched}"
    );
    let patch = read(&directory.join("Portfile.patch"));
    assert!(
        patch.contains("-revision 3") && patch.contains("+revision 0"),
        "{patch}"
    );
    assert_eq!(read(&directory.join("Portfile")), text);

    let output = tree.ok(&["bump", "bumped"]);
    let sha = "84d89877f0d4041efb6bf91a16f0248f2fd573e6af05c19f96bedb9f882f7882";
    assert_eq!(
        output,
        format!(
            "We will bump these:\nOld sha256:  {stale}\nNew sha256:  {sha}\nOld size:    10\nNew size:    10\nChecksums successfully bumped. Suggested commit message:\n        bumped: update to 2\n"
        )
    );
    let updated = read(&directory.join("Portfile"));
    assert!(
        updated.contains(&format!("checksums sha256 {sha} \\\n          size 10")),
        "{updated}"
    );
    assert!(updated.contains("revision 0\n"));
    assert_eq!(tree.ok(&["bump", "bumped"]), "No changes needed.\n");
}

#[test]
fn diagnose_reports_missing_archives_files_and_configuration() {
    let tree = Tree::new();
    let missing = tree.run(&["diagnose"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("No configuration file found at"));

    let resources = tree.root.join("ports/_resources/macports1.0");
    fs::create_dir_all(&resources).unwrap();
    fs::write(
        resources.join("xcode_versions.ini"),
        "# test\nxcode_version_1=1.0\n",
    )
    .unwrap();
    tree.ok(&["install", "lib"]);
    fs::remove_file(tree.file("share/lib/version")).unwrap();
    for entry in fs::read_dir(tree.root.join("db/software/lib")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let output = tree.run(&["diagnose"]);
    assert!(output.status.success());
    let progress = String::from_utf8_lossy(&output.stderr);
    assert!(
        progress.contains("Warning: No Xcode version info was found for your OS version."),
        "{progress}"
    );
    assert!(
        progress.contains(
            "couldn't find the archive for 'lib @1_0'. Please uninstall and reinstall this port."
        ),
        "{progress}"
    );
    assert!(
        progress.contains(&format!(
            "couldn't find file '{}' for port 'lib'.",
            tree.file("share/lib/version").display()
        )),
        "{progress}"
    );
}

/// Serve fixed files by path on a local port; other paths are 404.
fn serve_files(files: std::collections::HashMap<String, Vec<u8>>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..count]);
            let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
            match files.get(&path) {
                Some(body) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
            }
        }
    });
    format!("http://{address}/")
}

#[test]
fn install_prefers_signed_archives_and_honors_source_and_binary_only() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_compact::{KeyPair, Seed};
    let tree = Tree::new();
    tree.ok(&["-s", "archive", "lib"]);
    let image = fs::read_dir(tree.root.join("db/software/lib"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "tbz2"))
        .unwrap();
    let filename = image.file_name().unwrap().to_string_lossy().into_owned();
    let bytes = fs::read(&image).unwrap();
    tree.ok(&["uninstall", "lib"]);

    let key = KeyPair::from_seed(Seed::new([7; 32]));
    let envelope = |data: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(data);
        format!("untrusted comment: test\n{}\n", STANDARD.encode(packet))
    };
    let public = tree.root.join("archives.pub");
    fs::write(&public, envelope(key.pk.as_ref())).unwrap();
    let signature = envelope(key.sk.sign(&bytes, None).as_ref());
    let url = serve_files(std::collections::HashMap::from([
        (format!("/lib/{filename}"), bytes),
        (format!("/lib/{filename}.sig"), signature.into_bytes()),
    ]));
    let prefix = tree.root.join("prefix");
    let fetch = tree.root.join("ports/_resources/port1.0/fetch");
    fs::create_dir_all(&fetch).unwrap();
    fs::write(
        fetch.join("archive_sites.tcl"),
        format!(
            "namespace eval ::portfetch::mirror_sites {{\nvariable sites\nset sites(test_archives) {{{url}:nosubdir}}\nvariable archive_type\nset archive_type(test_archives) tbz2\nvariable archive_prefix\nset archive_prefix(test_archives) {p}\nvariable archive_frameworks_dir\nset archive_frameworks_dir(test_archives) {p}/Library/Frameworks\nvariable archive_applications_dir\nset archive_applications_dir(test_archives) {p}/Applications\nvariable archive_cxx_stdlib\nset archive_cxx_stdlib(test_archives) libc++\nvariable archive_delete_la_files\nset archive_delete_la_files(test_archives) yes\nvariable archive_sigtype\nset archive_sigtype(test_archives) sig\nvariable archive_pubkey\nset archive_pubkey(test_archives) {key}\n}}\n",
            p = prefix.display(),
            key = public.display()
        ),
    )
    .unwrap();

    let binary = tree.run(&["install", "lib"]);
    let progress = String::from_utf8_lossy(&binary.stderr);
    assert!(binary.status.success(), "{progress}");
    assert!(
        progress.contains("Fetching archive for lib") && !progress.contains("Building lib"),
        "{progress}"
    );
    assert_eq!(read(&tree.file("share/lib/version")), "lib-1\n");

    let missing = tree.run(&["-b", "install", "app"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("binary-only mode"));
    let source = tree.run(&["-s", "install", "app"]);
    let progress = String::from_utf8_lossy(&source.stderr);
    assert!(
        source.status.success() && progress.contains("Building app"),
        "{progress}"
    );
    assert!(!progress.contains("Fetching archive"), "{progress}");
}

#[test]
fn install_skips_build_dependencies_of_ports_with_archives() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_compact::{KeyPair, Seed};
    let tree = Tree::new();
    tree.port("tool", "1", "build { error \"tool must not be built\" }");
    tree.port("cli", "1", "depends_build port:tool\ndepends_lib port:lib");
    tree.ok(&["-s", "archive", "lib"]);
    let directory = tree.root.join("ports/devel/cli");
    let portfile = fs::read_to_string(directory.join("Portfile")).unwrap();
    fs::write(
        directory.join("Portfile"),
        portfile.replace("depends_build port:tool\n", ""),
    )
    .unwrap();
    tree.ok(&["-s", "archive", "cli"]);
    fs::write(directory.join("Portfile"), portfile).unwrap();
    let key = KeyPair::from_seed(Seed::new([9; 32]));
    let envelope = |data: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(data);
        format!("untrusted comment: test\n{}\n", STANDARD.encode(packet))
    };
    let mut files = std::collections::HashMap::new();
    for port in ["lib", "cli"] {
        let image = fs::read_dir(tree.root.join("db/software").join(port))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "tbz2"))
            .unwrap();
        let filename = image.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(&image).unwrap();
        let signature = envelope(key.sk.sign(&bytes, None).as_ref());
        files.insert(format!("/{port}/{filename}.sig"), signature.into_bytes());
        files.insert(format!("/{port}/{filename}"), bytes);
    }
    tree.ok(&["uninstall", "cli"]);
    tree.ok(&["uninstall", "lib"]);
    let public = tree.root.join("archives.pub");
    fs::write(&public, envelope(key.pk.as_ref())).unwrap();
    let url = serve_files(files);
    let prefix = tree.root.join("prefix");
    let fetch = tree.root.join("ports/_resources/port1.0/fetch");
    fs::create_dir_all(&fetch).unwrap();
    fs::write(
        fetch.join("archive_sites.tcl"),
        format!(
            "namespace eval ::portfetch::mirror_sites {{\nvariable sites\nset sites(test_archives) {{{url}:nosubdir}}\nvariable archive_type\nset archive_type(test_archives) tbz2\nvariable archive_prefix\nset archive_prefix(test_archives) {p}\nvariable archive_frameworks_dir\nset archive_frameworks_dir(test_archives) {p}/Library/Frameworks\nvariable archive_applications_dir\nset archive_applications_dir(test_archives) {p}/Applications\nvariable archive_cxx_stdlib\nset archive_cxx_stdlib(test_archives) libc++\nvariable archive_delete_la_files\nset archive_delete_la_files(test_archives) yes\nvariable archive_sigtype\nset archive_sigtype(test_archives) sig\nvariable archive_pubkey\nset archive_pubkey(test_archives) {key}\n}}\n",
            p = prefix.display(),
            key = public.display()
        ),
    )
    .unwrap();

    let output = tree.run(&["install", "cli"]);
    let progress = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{progress}");
    assert!(
        progress.contains("Fetching archive for cli")
            && progress.contains("Fetching archive for lib")
            && progress.contains("Fetching 2 signed archives (2 concurrent transfers)")
            && progress.contains("Preparing 2 archives (")
            && !progress.contains("tool"),
        "{progress}"
    );
    let installed = tree.ok(&["installed"]);
    assert!(installed.contains("cli @1_0 (active)"), "{installed}");
    assert!(!installed.contains("tool"), "{installed}");
}

#[test]
fn install_activates_images_with_more_files_than_the_default_descriptor_limit() {
    let tree = Tree::new();
    let directory = tree.root.join("ports/devel/many");
    tree.port("many", "1", "");
    fs::write(
        directory.join("Portfile"),
        portfile("many", "1", "").replace(
            "    system \"echo many-1",
            "    for {set i 0} {$i < 600} {incr i} {\n        xinstall -d ${destroot}${prefix}/share/many/d[expr {$i % 3}]\n        system \"touch ${destroot}${prefix}/share/many/d[expr {$i % 3}]/f$i\"\n    }\n    system \"echo many-1",
        ),
    )
    .unwrap();
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg("ulimit -Sn 256 && exec \"$0\" \"$@\"")
        .arg(env!("CARGO_BIN_EXE_port"))
        .arg("--index-file")
        .arg(tree.root.join("ports/PortIndex"))
        .args(["install", "many"])
        .env("PORTSRC", tree.root.join("macports.conf"))
        .env("HOME", &tree.root)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(tree.file("share/many/d2/f599").is_file());
}

#[test]
fn packages_build_installer_pkgs_meta_packages_and_disk_images() {
    let tree = Tree::new();
    tree.ok(&["pkg", "app"]);
    let work = tree
        .root
        .join("db/build")
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_devel_app"))
        .unwrap()
        .join("app/work");
    assert!(work.join("app-1.pkg").is_file());
    let payload = Command::new("/usr/sbin/pkgutil")
        .arg("--payload-files")
        .arg(work.join("app-1-component.pkg"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&payload.stdout).contains("share/app/version"));

    tree.ok(&["mpkg", "app"]);
    let listing = Command::new("/usr/bin/xar")
        .arg("-tf")
        .arg(work.join("app-1.mpkg"))
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout);
    assert!(
        listing.contains("lib-1-component.pkg") && listing.contains("app-1-component.pkg"),
        "{listing}"
    );

    tree.ok(&["dmg", "app"]);
    assert!(work.join("app-1.dmg").is_file());

    tree.ok(&["-s", "archive", "lib"]);
    tree.ok(&["clean", "lib"]);
    let output = tree.run(&["unarchive", "lib"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unpacking tbz2 archive for lib 1_0"));
    let lib = tree
        .root
        .join("db/build")
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_devel_lib"))
        .unwrap()
        .join("lib/work/destroot");
    let staged = lib.join(tree.file("share/lib/version").strip_prefix("/").unwrap());
    assert_eq!(read(&staged), "lib-1\n");
    assert!(!lib.join("+CONTENTS").exists());
}

#[test]
fn global_options_pseudo_portnames_paths_and_command_files() {
    let tree = Tree::new();
    let work = |port: &str| {
        tree.root
            .join("db/build")
            .read_dir()
            .ok()
            .and_then(|mut entries| {
                entries
                    .find(|entry| {
                        entry.as_ref().is_ok_and(|e| {
                            e.path()
                                .to_string_lossy()
                                .ends_with(&format!("_devel_{port}"))
                        })
                    })
                    .map(|entry| entry.unwrap().path().join(port).join("work"))
            })
    };

    // -n installs only the requested port; -k keeps its work directory.
    tree.ok(&["-n", "-k", "install", "app"]);
    assert!(tree.ok(&["installed"]).contains("app @1_0 (active)"));
    assert!(!tree.ok(&["installed"]).contains("lib @"));
    assert!(work("app").is_some_and(|work| work.exists()));
    tree.ok(&["uninstall", "app"]);

    // Autoclean is the default after a successful install.
    tree.ok(&["install", "app"]);
    assert!(!work("app").is_some_and(|work| work.exists()));

    let echo = |selector: &str| {
        tree.ok(&["echo", selector])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(echo("installed"), "app lib");
    assert_eq!(echo("requested"), "app");
    assert_eq!(echo("unrequested"), "lib");
    assert_eq!(echo("leaves"), "");

    // -y skips state changes.
    let dry = tree.run(&["-y", "uninstall", "app"]);
    assert!(String::from_utf8_lossy(&dry.stderr).contains("Skipping uninstall (dry run)"));
    assert!(tree.ok(&["installed"]).contains("app @1_0 (active)"));

    // `current`, `.` and directory paths name the port in that directory.
    let directory = tree.root.join("ports/devel/lib");
    let in_directory = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .arg("--index-file")
            .arg(tree.root.join("ports/PortIndex"))
            .args(args)
            .current_dir(&directory)
            .env("PORTSRC", tree.root.join("macports.conf"))
            .env("HOME", &tree.root)
            .output()
            .unwrap()
    };
    for selector in ["current", ".", "../lib"] {
        let output = in_directory(&["info", "--name", selector]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "name: lib\n",
            "{selector}"
        );
    }
    assert_eq!(
        tree.ok(&["-D", "lib", "info", "--name", "current"]),
        "name: lib\n"
    );

    // -F runs a command file; `cd` changes to a port; `quit` stops.
    let commands = tree.root.join("commands.txt");
    fs::write(
        &commands,
        "# comment\necho requested\ncd lib\ninfo --name current ; info --version app\nquit\necho installed\n",
    )
    .unwrap();
    assert_eq!(
        tree.ok(&["-F", commands.to_str().unwrap()])
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>(),
        ["app", "name: lib", "version: 1"]
    );

    // With no action, commands come from standard input.
    let mut shell = Command::new(env!("CARGO_BIN_EXE_port"))
        .arg("--index-file")
        .arg(tree.root.join("ports/PortIndex"))
        .env("PORTSRC", tree.root.join("macports.conf"))
        .env("HOME", &tree.root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    shell
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo unrequested\nexit\n")
        .unwrap();
    let output = shell.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "lib");

    // -p keeps going after a failing action and still reports failure.
    let output = tree.run(&["-p", "info", "nope", ";", "info", "--name", "lib"]);
    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "name: lib\n");
    let output = tree.run(&["info", "nope", ";", "info", "--name", "lib"]);
    assert!(output.stdout.is_empty());

    // -d shows debug messages; inactive images are selectable by name.
    let debug = tree.run(&["-d", "destroot", "lib"]);
    assert!(String::from_utf8_lossy(&debug.stderr).contains("DEBUG: staging lib"));
    tree.port("app", "2", "depends_lib port:lib");
    assert_eq!(echo("outdated"), "app");
    tree.ok(&["upgrade", "outdated"]);
    assert_eq!(echo("inactive"), "app");
    tree.ok(&["uninstall", "inactive"]);
    assert_eq!(echo("inactive"), "");
}

#[test]
fn activation_hooks_resolve_dependencies_and_deactivate_replaced_ports() {
    let tree = Tree::new();
    tree.ok(&["install", "lib"]);
    tree.port(
        "new",
        "1",
        r#"pre-activate {
    if {[_get_dep_port port:lib] ne "lib"} { error "lib not resolved" }
    if {![catch {set installed [lindex [registry_active lib] 0]}]} {
        registry_deactivate_composite lib "" [list ports_nodepcheck 1]
    }
}"#,
    );
    tree.ok(&["install", "new"]);
    let list = tree.ok(&["installed"]);
    assert!(
        list.contains("new @1_0 (active)") && list.contains("lib @1_0\n"),
        "{list}"
    );
    assert!(!tree.file("share/lib/version").exists());
}
