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
        for port in ["lib", "app"] {
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
