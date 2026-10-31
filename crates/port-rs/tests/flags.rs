//! Action flags from upstream's per-action option table, end to end against a
//! private prefix and registry.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Tree {
    _temp: tempfile::TempDir,
    root: PathBuf,
    ports: Vec<(String, String)>,
}

fn portfile(name: &str, extra: &str) -> String {
    format!(
        r#"PortSystem 1.0
name {name}
version 1
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
build {{}}
destroot {{
    xinstall -d ${{destroot}}${{prefix}}/share/{name}
    system "echo {name} > ${{destroot}}${{prefix}}/share/{name}/version"
}}
{extra}
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
                "prefix {0}/prefix\nportdbpath {0}/db\napplications_dir {0}/prefix/Applications\nframeworks_dir {0}/prefix/Library/Frameworks\nplace_worksymlink no\nsandbox_enable no\nsources_conf {0}/sources.conf\n",
                root.display()
            ),
        )
        .unwrap();
        fs::write(root.join("sources.conf"), "").unwrap();
        Self {
            _temp: temp,
            root,
            ports: Vec::new(),
        }
    }

    /// Add a port; `fields` are extra PortIndex fields, `extra` Portfile lines.
    fn port(&mut self, name: &str, fields: &str, extra: &str) -> &mut Self {
        let directory = self.root.join("ports/devel").join(name);
        fs::create_dir_all(&directory).unwrap();
        let depends = fields
            .split_once("depends_lib ")
            .map(|(_, rest)| format!("depends_lib {}", rest.split_whitespace().next().unwrap()))
            .unwrap_or_default();
        fs::write(
            directory.join("Portfile"),
            portfile(name, &format!("{depends}\n{extra}")),
        )
        .unwrap();
        self.ports.push((name.into(), fields.into()));
        let mut index = String::new();
        for (port, fields) in &self.ports {
            let body = format!("name {port} portdir devel/{port} version 1 revision 0 {fields}");
            index.push_str(&format!("{port} {}\n{body}\n", body.len() + 1));
        }
        fs::write(self.root.join("ports/PortIndex"), index).unwrap();
        self
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
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{args:?}\n{text}");
        text
    }

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn installed(&self) -> String {
        self.ok(&["-q", "installed"])
    }

    fn prefix(&self) -> PathBuf {
        self.root.join("prefix")
    }
}

fn exists(path: &Path) -> bool {
    path.try_exists().unwrap()
}

#[test]
fn install_flags_control_activation_requested_status_replacements_and_known_failures() {
    let mut tree = Tree::new();
    tree.port("lib", "", "")
        .port("old", "replaced_by new", "")
        .port("new", "", "")
        .port("broken", "known_fail yes", "");

    let output = tree.ok(&["install", "--unrequested", "lib"]);
    assert!(output.contains("No broken files found"), "{output}");
    assert!(tree.ok(&["-q", "installed", "requested"]).trim().is_empty());
    assert!(tree.installed().contains("lib @1_0 (active)"));

    let output = tree.ok(&["install", "--no-rev-upgrade", "old"]);
    assert!(output.contains("old is replaced by new"), "{output}");
    assert!(!output.contains("No broken files found"), "{output}");
    let installed = tree.installed();
    assert!(installed.contains("new @1_0 (active)"), "{installed}");
    assert!(!installed.contains("old @"), "{installed}");
    tree.ok(&["install", "--no-replace", "--no-activate", "old"]);
    let installed = tree.installed();
    assert!(
        installed.contains("old @1_0\n") || installed.ends_with("old @1_0"),
        "{installed}"
    );
    assert!(!exists(&tree.prefix().join("share/old")));

    let error = tree.fails(&["install", "broken"]);
    assert!(
        error.contains("broken is known to fail (use --allow-failing"),
        "{error}"
    );
    tree.ok(&["install", "--allow-failing", "broken"]);
    assert!(exists(&tree.prefix().join("share/broken/version")));
}

#[test]
fn lifecycle_actions_run_saved_portfile_hooks_unless_no_exec() {
    let mut tree = Tree::new();
    let marker = |name: &str| format!("system \"echo {name} >> ${{portdbpath}}/../hooks\"");
    tree.port(
        "hooked",
        "",
        &format!(
            "pre-activate {{ {} }}\npost-deactivate {{ {} }}\npre-uninstall {{ {} }}",
            marker("activate"),
            marker("deactivate"),
            marker("uninstall")
        ),
    );
    let hooks = tree.root.join("hooks");
    tree.ok(&["install", "hooked"]);
    let saved: Vec<_> = fs::read_dir(tree.root.join("db/registry/portfiles/hooked-1_0"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1);
    assert!(exists(&saved[0].join("Portfile")));
    // Hooks come from the saved Portfile, even after the tree's copy changes.
    fs::write(
        tree.root.join("ports/devel/hooked/Portfile"),
        portfile("hooked", ""),
    )
    .unwrap();
    fs::remove_file(&hooks).unwrap();

    tree.ok(&["deactivate", "hooked"]);
    assert_eq!(fs::read_to_string(&hooks).unwrap(), "deactivate\n");
    tree.ok(&["activate", "hooked"]);
    assert_eq!(
        fs::read_to_string(&hooks).unwrap(),
        "deactivate\nactivate\n"
    );
    tree.ok(&["deactivate", "--no-exec", "hooked"]);
    tree.ok(&["activate", "--no-exec", "hooked"]);
    assert_eq!(
        fs::read_to_string(&hooks).unwrap(),
        "deactivate\nactivate\n"
    );

    tree.ok(&["uninstall", "hooked"]);
    assert_eq!(
        fs::read_to_string(&hooks).unwrap(),
        "deactivate\nactivate\nuninstall\ndeactivate\n"
    );
    assert!(!exists(&tree.root.join("db/registry/portfiles/hooked-1_0")));

    tree.ok(&["install", "hooked"]);
    fs::remove_file(&hooks).unwrap();
    tree.ok(&["uninstall", "--no-exec", "hooked"]);
    assert!(!exists(&hooks));
    assert!(tree.installed().trim().is_empty() || !tree.installed().contains("hooked"));
}

#[test]
fn uninstall_follows_dependents_and_unneeded_dependencies() {
    let mut tree = Tree::new();
    tree.port("base", "", "")
        .port("lib", "depends_lib port:base", "")
        .port("app", "depends_lib port:lib", "")
        .port("tool", "depends_lib port:base", "");

    tree.ok(&["install", "app"]);
    let error = tree.fails(&["uninstall", "lib"]);
    assert!(
        error.contains("the following ports depend on it"),
        "{error}"
    );
    tree.ok(&["uninstall", "--follow-dependents", "lib"]);
    let installed = tree.installed();
    assert!(
        !installed.contains("lib @") && !installed.contains("app @"),
        "{installed}"
    );
    assert!(installed.contains("base @1_0"), "{installed}");

    tree.ok(&["install", "app", "tool"]);
    let output = tree.ok(&["uninstall", "--follow-dependencies", "app"]);
    assert!(
        output.contains("The following dependencies will be uninstalled"),
        "{output}"
    );
    let installed = tree.installed();
    // base stays: tool still needs it.
    assert!(
        !installed.contains("lib @") && !installed.contains("app @"),
        "{installed}"
    );
    assert!(
        installed.contains("base @1_0") && installed.contains("tool @1_0"),
        "{installed}"
    );

    tree.ok(&["uninstall", "--follow-dependencies", "tool"]);
    assert!(!tree.installed().contains("base @"), "{}", tree.installed());
}

#[test]
fn upgrade_forces_rebuilds_replaces_ports_and_enforces_variants() {
    let mut tree = Tree::new();
    tree.port(
        "lib",
        "variants extra",
        "variant extra description {extra} {}",
    )
    .port("old", "replaced_by new", "")
    .port("new", "", "");
    tree.ok(&["install", "lib"]);
    tree.ok(&["install", "--no-replace", "old"]);

    assert!(tree.ok(&["upgrade", "lib"]).contains("Nothing to upgrade"));
    let output = tree.ok(&["upgrade", "--force", "--no-rev-upgrade", "lib"]);
    assert!(output.contains("Uninstalling lib @1_0"), "{output}");
    assert!(output.contains("Installed lib @1_0"), "{output}");
    assert!(!output.contains("No broken files found"), "{output}");
    assert!(tree.installed().contains("lib @1_0 (active)"));

    let output = tree.ok(&["upgrade", "lib", "+extra"]);
    assert!(
        output.contains("Use 'upgrade --enforce-variants'"),
        "{output}"
    );
    assert!(tree.installed().contains("lib @1_0 (active)"));
    tree.ok(&["upgrade", "--enforce-variants", "lib", "+extra"]);
    assert!(
        tree.installed().contains("lib @1_0+extra (active)"),
        "{}",
        tree.installed()
    );

    assert!(
        tree.ok(&["upgrade", "--no-replace", "old"])
            .contains("Nothing to upgrade")
    );
    let output = tree.ok(&["upgrade", "old"]);
    assert!(output.contains("old is replaced by new"), "{output}");
    let installed = tree.installed();
    assert!(installed.contains("new @1_0 (active)"), "{installed}");
    assert!(
        installed.contains("old @1_0") && !installed.contains("old @1_0 (active)"),
        "{installed}"
    );
    assert!(
        tree.ok(&["-q", "installed", "requested"])
            .contains("new @1_0")
    );
}

#[test]
fn fetch_no_mirrors_reaches_the_portfile_and_edit_takes_an_editor() {
    let mut tree = Tree::new();
    tree.port(
        "lib",
        "",
        "pre-fetch { ui_msg \"no-mirrors=[info exists ports_fetch_no-mirrors]\" }",
    );
    assert!(tree.ok(&["fetch", "lib"]).contains("no-mirrors=0"));
    tree.ok(&["clean", "lib"]);
    assert!(
        tree.ok(&["fetch", "--no-mirrors", "lib"])
            .contains("no-mirrors=1")
    );
    let output = tree.ok(&["edit", "--editor", "/bin/echo editing", "lib"]);
    assert!(
        output.contains("editing ") && output.contains("devel/lib/Portfile"),
        "{output}"
    );
}
