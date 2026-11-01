use port_rs::{
    index::Port,
    install::{
        Action, Item,
        archive::VerifiedArchive,
        binary::Installer,
        inspect::Limits,
        source::package::{self, Inputs},
        store::Store,
        target::Target,
    },
    progress::Progress,
    registry::write::{NewImage, Writer},
    source::Platform,
};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

fn target(name: &str) -> Target {
    let mut session = port_rs::portfile::Session::open(
        &format!("name {name}; version 1; platforms darwin; set configure.build_arch arm64; set os.arch arm"),
        &BTreeMap::new(),
        &port_rs::spec::PortSpec::named(name),
        port_rs::portfile::PortGroups::default(),
    )
    .unwrap();
    Target::from_session(
        &mut session,
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
    )
    .unwrap()
}

/// Package and install `name` with one file under `prefix`, depending on `deps`.
fn install(root: &Path, prefix: &Path, name: &str, deps: &[&str]) {
    let destroot = root.join(format!("destroot-{name}"));
    let payload = destroot
        .join(prefix.strip_prefix("/").unwrap())
        .join(format!("share/{name}"));
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("data.txt"), name).unwrap();
    fs::write(root.join("state"), "").unwrap();
    fs::write(root.join("Portfile"), "").unwrap();
    let output = root.join(format!("{name}.tbz2"));
    package::create(
        &Inputs {
            destroot,
            statefile: root.join("state"),
            portfile: root.join("Portfile"),
            subport: name.into(),
            version: "1".into(),
            archs: "arm64".into(),
            source_date_epoch: "0".into(),
            macports_version: "2.12.7".into(),
            ..Inputs::default()
        },
        &output,
    )
    .unwrap();
    let port = Port {
        name: name.into(),
        fields: BTreeMap::from([
            ("version".into(), "1".into()),
            ("revision".into(), "0".into()),
        ]),
    };
    let item = Item {
        port: &port,
        variants: String::new(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    };
    fs::create_dir_all(root.join("software")).unwrap();
    let store = Store::open(&root.join("software")).unwrap();
    let registry = root.join("registry.db");
    let mut writer = if registry.exists() {
        Writer::open(&registry)
    } else {
        Writer::create(&registry)
    }
    .unwrap();
    let staging = tempfile::tempdir().unwrap();
    let progress = Progress::new(true);
    Installer {
        store: &store,
        registry: &mut writer,
        staging: staging.path(),
        root: Path::new("/"),
        limits: Limits::default(),
        progress: &progress,
        prepared: None,
    }
    .install(
        &mut VerifiedArchive::local(fs::File::open(&output).unwrap()).unwrap(),
        &item,
        &target(name),
        NewImage {
            name: name.into(),
            version: "1".into(),
            archs: "arm64".into(),
            platform: "darwin".into(),
            os_major: "24".into(),
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            ..NewImage::default()
        },
    )
    .unwrap();
}

fn port(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_port"))
        .arg("--registry-file")
        .arg(root.join("registry.db"))
        .args(args)
        .env("HOME", root)
        .output()
        .unwrap()
}

#[test]
fn deactivate_activate_and_uninstall_respect_dependents() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let prefix = root.join("prefix");
    install(&root, &prefix, "app", &[]);
    install(&root, &prefix, "dep", &["app"]);
    let app_file = prefix.join("share/app/data.txt");
    assert_eq!(fs::read_to_string(&app_file).unwrap(), "app");

    let blocked = port(&root, &["-q", "deactivate", "app"]);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("dep @1_0"));
    assert!(app_file.exists());

    assert!(port(&root, &["-q", "deactivate", "dep"]).status.success());
    assert!(
        !prefix.join("share/dep").exists(),
        "emptied directories are pruned"
    );
    assert!(port(&root, &["-q", "deactivate", "app"]).status.success());
    assert!(!app_file.exists());
    let again = port(&root, &["-q", "deactivate", "app"]);
    assert!(String::from_utf8_lossy(&again.stderr).contains("is not active"));

    assert!(port(&root, &["-q", "activate", "app"]).status.success());
    assert_eq!(fs::read_to_string(&app_file).unwrap(), "app");
    let installed = port(&root, &["-q", "installed"]);
    assert!(String::from_utf8_lossy(&installed.stdout).contains("app @1_0 (active)"));

    let refused = port(&root, &["-q", "uninstall", "app"]);
    assert!(
        !refused.status.success(),
        "inactive dependents still block uninstall"
    );
    assert!(port(&root, &["-q", "uninstall", "dep"]).status.success());
    assert!(port(&root, &["-q", "uninstall", "app"]).status.success());
    assert!(!app_file.exists());
    assert!(!root.join("software/app").exists());
    let empty = port(&root, &["installed"]);
    assert!(String::from_utf8_lossy(&empty.stdout).contains("No ports are installed"));
}

#[test]
fn forced_deactivation_warns_but_proceeds() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let prefix = root.join("prefix");
    install(&root, &prefix, "app", &[]);
    install(&root, &prefix, "dep", &["app"]);
    let forced = port(&root, &["deactivate", "-f", "app"]);
    assert!(
        forced.status.success(),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert!(!prefix.join("share/app/data.txt").exists());
    assert!(prefix.join("share/dep/data.txt").exists());
}
