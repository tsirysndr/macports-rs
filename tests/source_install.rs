//! Offline end-to-end source installs through the CLI, with Portfiles that
//! need no downloads, a private prefix and a private registry.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

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
                "prefix {0}/prefix\nportdbpath {0}/db\napplications_dir {0}/prefix/Applications\nframeworks_dir {0}/prefix/Library/Frameworks\nplace_worksymlink no\n",
                root.display()
            ),
        )
        .unwrap();
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
            let depends = if port == "app" {
                " depends_lib port:lib"
            } else {
                ""
            };
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

    tree.ok(&["setunrequested", "app"]);
    let output = tree.run(&["reclaim"]);
    let progress = String::from_utf8_lossy(&output.stderr);
    let app = progress.find("Uninstalling app @2_0").expect(&progress);
    let lib = progress.find("Uninstalling lib @1_0").expect(&progress);
    assert!(app < lib, "{progress}");
    assert!(tree.ok(&["installed"]).contains("No ports are installed"));

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
    tree.ok(&["install", "lib", "linked"]);
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
