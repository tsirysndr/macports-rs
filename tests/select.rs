use std::{fs, path::Path, process::Command};

fn port(root: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_port"))
        .args(args)
        .env("PORTSRC", root.join("macports.conf"))
        .env("HOME", root)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn select_lists_sets_shows_and_summarizes_groups() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let prefix = root.join("prefix");
    let group = prefix.join("etc/select/python");
    fs::create_dir_all(&group).unwrap();
    fs::write(
        root.join("macports.conf"),
        format!("prefix {}\n", prefix.display()),
    )
    .unwrap();
    fs::write(group.join("base"), "bin/python\nbin/pydoc\n").unwrap();
    fs::write(group.join("python312"), "bin/python3.12\n-\n").unwrap();
    fs::write(group.join("python313"), "bin/python3.13\n/usr/bin/pydoc3\n").unwrap();
    fs::write(group.join("none"), "-\n-\n").unwrap();

    let (ok, out, _) = port(&root, &["select", "python"]);
    assert!(ok);
    assert_eq!(
        out,
        "Available versions for python:\n\tnone (active)\n\tpython312\n\tpython313\n"
    );
    let (ok, out, _) = port(&root, &["select", "--show", "python"]);
    assert!(ok);
    assert_eq!(
        out,
        "The currently selected version for 'python' is 'none'.\n"
    );

    let (ok, out, _) = port(&root, &["select", "--set", "python", "python313"]);
    assert!(ok, "{out}");
    assert_eq!(
        out,
        "Selecting 'python313' for 'python' succeeded. 'python313' is now active.\n"
    );
    assert_eq!(
        fs::read_link(prefix.join("bin/python")).unwrap(),
        prefix.join("bin/python3.13")
    );
    assert_eq!(
        fs::read_link(prefix.join("bin/pydoc")).unwrap(),
        Path::new("/usr/bin/pydoc3")
    );

    let (ok, _, _) = port(&root, &["select", "python", "python312"]);
    assert!(ok);
    assert!(fs::symlink_metadata(prefix.join("bin/pydoc")).is_err());
    let (_, out, _) = port(&root, &["select", "--list", "python"]);
    assert!(out.contains("\tpython312 (active)\n"), "{out}");

    let (ok, out, _) = port(&root, &["select", "--summary"]);
    assert!(ok);
    assert_eq!(
        out,
        "Name    Selected   Options\n====    ========   =======\npython  python312  python312 python313 none\n"
    );

    let (ok, out, _) = port(&root, &["select", "--set", "python", "base"]);
    assert!(!ok);
    assert!(
        out.contains("failed: The specified version 'base' is not valid."),
        "{out}"
    );
    let (ok, _, err) = port(&root, &["select", "missing"]);
    assert!(
        !ok && err.contains("The specified group 'missing' does not exist."),
        "{err}"
    );
    let (ok, _, err) = port(&root, &["select", "--list", "--show", "python"]);
    assert!(!ok && err.contains("Multiple commands"), "{err}");
}
