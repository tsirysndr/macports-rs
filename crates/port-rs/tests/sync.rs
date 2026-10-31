use std::{fs, path::Path, process::Command};

fn git(directory: &Path, args: &[&str]) {
    let status = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(directory)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn index(version: &str) -> String {
    let body = format!("name zlib portdir archivers/zlib version {version} revision 0");
    format!("zlib {}\n{body}\n", body.len() + 1)
}

/// A tree whose Portfile sync indexes, as upstream reindexes file:// sources.
fn tree(directory: &Path, version: &str) {
    let port = directory.join("archivers/zlib");
    fs::create_dir_all(&port).unwrap();
    fs::create_dir_all(directory.join("_resources/port1.0/group")).unwrap();
    fs::write(directory.join("_resources/port1.0/group/.keep"), "").unwrap();
    fs::write(
        port.join("Portfile"),
        format!("PortSystem 1.0\nname zlib\nversion {version}\ncategories archivers\nmaintainers nomaintainer\ndescription zlib\nlong_description zlib\nhomepage https://zlib.net\nplatforms darwin\nlicense Zlib\n"),
    )
    .unwrap();
    fs::write(directory.join("PortIndex"), index(version)).unwrap();
}

#[test]
fn sync_pulls_file_sources_skips_nosync_and_selects_the_default_tree() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let upstream = root.join("upstream");
    fs::create_dir_all(&upstream).unwrap();
    tree(&upstream, "1");
    git(&upstream, &["init", "-q"]);
    git(&upstream, &["add", "."]);
    git(&upstream, &["commit", "-qm", "one"]);
    let checkout = root.join("checkout");
    let status = Command::new("/usr/bin/git")
        .args(["clone", "-q"])
        .arg(&upstream)
        .arg(&checkout)
        .status()
        .unwrap();
    assert!(status.success());
    tree(&upstream, "2");
    git(&upstream, &["commit", "-qam", "two"]);
    fs::write(
        root.join("sources.conf"),
        format!(
            "# comment\nfile://{0}/missing [nosync]\nfile://{0}/checkout [default]\n",
            root.display()
        ),
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
    let port = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(&root)
            .env("PORTSRC", root.join("macports.conf"))
            .env("HOME", &root)
            .output()
            .unwrap()
    };
    let before = port(&["info", "--version", "zlib"]);
    assert_eq!(String::from_utf8_lossy(&before.stdout), "version: 1\n");
    let synced = port(&["sync"]);
    assert!(
        synced.status.success(),
        "{}",
        String::from_utf8_lossy(&synced.stderr)
    );
    assert!(
        String::from_utf8_lossy(&synced.stdout).contains("Updating the ports tree")
            || String::from_utf8_lossy(&synced.stderr).contains("Updating the ports tree")
    );
    let after = port(&["info", "--version", "zlib"]);
    assert_eq!(String::from_utf8_lossy(&after.stdout), "version: 2\n");
    // A local release endpoint with no published releases.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let releases = format!("http://{}/latest", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let _ = stream.read(&mut [0; 4096]);
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let updated = Command::new(env!("CARGO_BIN_EXE_port"))
        .args(["selfupdate", "--nosync"])
        .current_dir(&root)
        .env("PORTSRC", root.join("macports.conf"))
        .env("HOME", &root)
        .env("PORT_RS_RELEASES_URL", &releases)
        .output()
        .unwrap();
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
}
