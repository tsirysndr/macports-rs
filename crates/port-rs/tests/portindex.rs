//! `portindex` (the binary invoked under that name) and `sync` reindexing
//! file:// sources that have no PortIndex.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn portfile(name: &str, extra: &str) -> String {
    format!(
        "PortSystem 1.0\nname {name}\nversion 1.2\nrevision 3\ncategories devel net\nplatforms darwin\nlicense MIT\nmaintainers nomaintainer\ndescription {name} library\nlong_description {name} for tests\nhomepage https://example.org\ndepends_lib port:zlib path:${{prefix}}/lib/libssl.dylib:openssl\n{extra}\n"
    )
}

struct Tree {
    _temp: tempfile::TempDir,
    root: PathBuf,
    tool: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let ports = root.join("ports");
        fs::create_dir_all(ports.join("_resources/port1.0/group")).unwrap();
        for (directory, text) in [
            ("devel/alpha", portfile("alpha", "")),
            (
                "devel/beta",
                portfile(
                    "beta",
                    "subport beta-extra { description extra beta }\nvariant x description {x} {}",
                ),
            ),
            (
                "net/gamma",
                portfile("gamma", "known_fail yes\nreplaced_by alpha"),
            ),
            ("net/broken", "PortSystem 1.0\nname broken\n".into()),
        ] {
            fs::create_dir_all(ports.join(directory)).unwrap();
            fs::write(ports.join(directory).join("Portfile"), text).unwrap();
        }
        fs::create_dir_all(ports.join("_private/ignored")).unwrap();
        fs::write(
            ports.join("_private/ignored/Portfile"),
            portfile("ignored", ""),
        )
        .unwrap();
        fs::write(
            root.join("macports.conf"),
            format!(
                "prefix {0}/prefix\nportdbpath {0}/db\nsources_conf {0}/sources.conf\n",
                root.display()
            ),
        )
        .unwrap();
        fs::write(
            root.join("sources.conf"),
            format!("file://{} [default]\n", ports.display()),
        )
        .unwrap();
        let tool = root.join("portindex");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_port"), &tool).unwrap();
        Self {
            _temp: temp,
            root,
            tool,
        }
    }

    fn portindex(&self, args: &[&str]) -> Output {
        Command::new(&self.tool)
            .args(args)
            .arg(self.root.join("ports"))
            .env("PORTSRC", self.root.join("macports.conf"))
            .env("HOME", &self.root)
            .output()
            .unwrap()
    }

    fn port(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(&self.root)
            .env("PORTSRC", self.root.join("macports.conf"))
            .env("HOME", &self.root)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
}

fn entries(index: &Path) -> Vec<(String, std::collections::BTreeMap<String, String>)> {
    let text = fs::read_to_string(index).unwrap();
    let mut lines = text.lines();
    let mut entries = Vec::new();
    while let (Some(header), Some(line)) = (lines.next(), lines.next()) {
        let (name, length) = header.split_once(' ').unwrap();
        assert_eq!(length.parse::<usize>().unwrap(), line.len() + 1, "{header}");
        entries.push((
            name.to_string(),
            port_rs::tcl_list::dictionary(line).unwrap(),
        ));
    }
    entries
}

#[test]
fn portindex_indexes_ports_and_subports_keeping_prefix_literal() {
    let tree = Tree::new();
    let output = tree.portindex(&["-e"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains("Total number of ports parsed:\t4"),
        "{stdout}"
    );
    assert!(stdout.contains("Ports failed:\t\t\t1"), "{stdout}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("net/broken/Portfile"));

    let index = tree.root.join("ports/PortIndex");
    let entries = entries(&index);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta", "beta-extra", "gamma"]);
    let alpha = &entries[0].1;
    assert_eq!(alpha["portdir"], "devel/alpha");
    assert_eq!(alpha["version"], "1.2");
    assert_eq!(alpha["revision"], "3");
    assert_eq!(alpha["categories"], "devel net");
    assert_eq!(
        alpha["depends_lib"],
        "port:zlib path:${prefix}/lib/libssl.dylib:openssl"
    );
    assert!(!alpha.contains_key("subports"));
    assert_eq!(entries[1].1["subports"], "beta-extra");
    assert_eq!(entries[1].1["variants"], "x");
    assert_eq!(entries[2].1["description"], "extra beta");
    assert!(!entries[2].1.contains_key("subports"));
    assert_eq!(entries[3].1["known_fail"], "yes");
    assert_eq!(entries[3].1["replaced_by"], "alpha");

    let quick = fs::read_to_string(tree.root.join("ports/PortIndex.quick")).unwrap();
    let text = fs::read_to_string(&index).unwrap();
    for line in quick.lines() {
        let (name, offset) = line.split_once(' ').unwrap();
        let offset: usize = offset.parse().unwrap();
        assert!(
            text[offset..]
                .to_lowercase()
                .starts_with(&format!("{name} ")),
            "{line}"
        );
    }

    // Unchanged Portfiles are reused; a changed one is evaluated again.
    let output = tree.portindex(&[]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Up-to-date ports skipped:\t3"));
    let alpha = tree.root.join("ports/devel/alpha/Portfile");
    fs::write(&alpha, portfile("alpha", "").replace("1.2", "1.3")).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&alpha)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let output = tree.portindex(&[]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Up-to-date ports skipped:\t2"));
    let entries = self::entries(&index);
    assert_eq!(entries[0].1["version"], "1.3");
    assert_eq!(entries.len(), 4);
    let output = tree.portindex(&["-f"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Up-to-date ports skipped:\t0"));
}

#[test]
fn sync_indexes_file_sources_and_ports_resolve_through_it() {
    let tree = Tree::new();
    let output = tree.port(&["sync"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(tree.root.join("ports/PortIndex").is_file());
    let output = tree.port(&["info", "--version", "beta-extra"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "version: 1.2\n");
}
