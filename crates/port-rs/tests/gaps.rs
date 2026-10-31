//! End-to-end tests for build sandboxing, trace mode, the mirror filemap,
//! selfupdate, RSA-signed archives, non-flat packages and out-of-tree ports.
#![allow(dead_code)]
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

fn add_port(tree: &Tree, name: &str, text: &str) {
    let directory = tree.root.join("ports/devel").join(name);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("Portfile"), text).unwrap();
    tree.port("lib", "1", "");
}

fn config(tree: &Tree, line: &str) {
    let path = tree.root.join("macports.conf");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).unwrap();
}

#[test]
fn sandbox_network_denies_network_access_during_builds() {
    let tree = Tree::new();
    let url = serve("ok");
    add_port(
        &tree,
        "online",
        &portfile("online", "1", "").replace(
            "build {}",
            &format!("build {{ system \"/usr/bin/curl -sf {url} -o /dev/null\" }}"),
        ),
    );
    tree.ok(&["build", "online"]);
    tree.ok(&["clean", "online"]);
    config(&tree, "sandbox_network yes");
    let denied = tree.run(&["build", "online"]);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("Failed to build online"));
}

#[test]
fn trace_mode_hides_files_of_undeclared_dependencies() {
    let tree = Tree::new();
    tree.ok(&["install", "lib"]);
    let reader = |depends: &str| {
        portfile("reader", "1", depends).replace(
            "build {}",
            "build { system \"cat ${prefix}/share/stray >/dev/null && cat ${prefix}/share/lib/version > ${workpath}/copy\" }",
        )
    };
    // A file in the prefix that no port installed.
    let stray = tree.root.join("prefix/share/stray");
    fs::write(&stray, "stray").unwrap();
    add_port(&tree, "reader", &reader(""));
    tree.ok(&["build", "reader"]);
    tree.ok(&["clean", "reader"]);
    let traced = tree.run(&["-t", "build", "reader"]);
    assert!(!traced.status.success(), "lib is not a declared dependency");
    let report = String::from_utf8_lossy(&traced.stderr);
    let hidden = tree.root.join("prefix/share/lib/version");
    assert!(
        report.contains(
            "The following existing file was hidden from the build system by trace mode:"
        ) && report.contains(&format!("  {}", hidden.display())),
        "{report}"
    );
    assert!(
        report.contains(
            "The following file inside the MacPorts prefix not installed by a port was accessed:"
        ) && report.contains(&format!("  {}", stray.display())),
        "{report}"
    );
    tree.ok(&["clean", "reader"]);
    add_port(&tree, "reader", &reader("depends_build port:lib"));
    let output = tree.ok(&["-t", "build", "reader"]);
    assert!(!output.contains("hidden from the build system"), "{output}");
}

#[test]
fn mirror_records_distfiles_in_the_upstream_filemap() {
    let tree = Tree::new();
    let url = serve("0123456789");
    let sha = "84d89877f0d4041efb6bf91a16f0248f2fd573e6af05c19f96bedb9f882f7882";
    add_port(
        &tree,
        "good",
        &portfile("good", "1", "").replace(
            "distfiles\n",
            &format!(
                "master_sites {url}\ndistfiles good-1.tar.gz\nchecksums sha256 {sha} size 10\n"
            ),
        ),
    );
    tree.ok(&["mirror", "good"]);
    let map = port_rs::filemap::Filemap::load(&tree.root.join("db/distfiles_mirror.db")).unwrap();
    let distfile = tree.root.join("db/distfiles/good/good-1.tar.gz");
    assert_eq!(map.get(&distfile.to_string_lossy()), Some("1"));
    let bytes = fs::read(tree.root.join("db/distfiles_mirror.db")).unwrap();
    assert!(bytes.starts_with(b"org.darwinports.filemap\0"));
}

/// Serve release metadata for `tag` with a `port` that prints "updated";
/// returns the releases URL.
fn serve_release(tag: &str) -> String {
    use sha2::Digest;
    let target = if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    };
    let mut archive = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut archive, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let payload = b"#!/bin/sh\necho updated\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "port", &payload[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }
    let digest: String = sha2::Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = format!("port-{target}.tar.gz");
    // Serve the release metadata and its assets from one server.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let root = format!("http://{address}/");
    let files = std::collections::HashMap::from([
        (
            "/latest".to_string(),
            format!(
                "{{\"tag_name\":\"v{tag}\",\"assets\":[{{\"name\":\"{name}\",\"browser_download_url\":\"{root}{name}\"}},{{\"name\":\"{name}.sha256\",\"browser_download_url\":\"{root}{name}.sha256\"}}]}}"
            )
            .into_bytes(),
        ),
        (format!("/{name}"), archive),
        (format!("/{name}.sha256"), format!("{digest}  {name}\n").into_bytes()),
    ]);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..count]);
            let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
            let body = files.get(&path).cloned().unwrap_or_default();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    format!("{root}latest")
}

/// Copy the port executable, then run `selfupdate --nosync` on the copy.
fn selfupdate_copy(tag: &str, args: &[&str]) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("port");
    fs::copy(env!("CARGO_BIN_EXE_port"), &binary).unwrap();
    let output = Command::new(&binary)
        .args(["selfupdate", "--nosync"])
        .args(args)
        .env("PORT_RS_RELEASES_URL", serve_release(tag))
        .env("HOME", temp.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    (temp, binary, stderr)
}

#[test]
fn selfupdate_replaces_the_port_executable_with_a_newer_release() {
    let (_temp, binary, stderr) = selfupdate_copy("99.0.0", &[]);
    assert!(stderr.contains("updated from"), "{stderr}");
    let updated = Command::new(&binary).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&updated.stdout), "updated\n");
}

#[test]
fn selfupdate_migrate_reinstalls_the_current_release() {
    let current = env!("CARGO_PKG_VERSION");
    let (_temp, binary, stderr) = selfupdate_copy(current, &[]);
    assert!(stderr.contains("already the latest version"), "{stderr}");
    assert!(
        Command::new(&binary)
            .arg("version")
            .output()
            .unwrap()
            .status
            .success()
    );
    let (_temp, binary, stderr) = selfupdate_copy(current, &["--migrate"]);
    assert!(stderr.contains("updated from"), "{stderr}");
    let updated = Command::new(&binary).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&updated.stdout), "updated\n");
}

#[test]
fn rsa_signed_archive_sites_install_verified_archives() {
    let tree = Tree::new();
    tree.ok(&["-s", "archive", "lib"]);
    let image = fs::read_dir(tree.root.join("db/software/lib"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "tbz2"))
        .unwrap();
    let filename = image.file_name().unwrap().to_string_lossy().into_owned();
    let staged = tree.root.join(&filename);
    fs::copy(&image, &staged).unwrap();
    tree.ok(&["uninstall", "lib"]);
    let key = tree.root.join("key.pem");
    let public = tree.root.join("public.pem");
    let signature = tree.root.join("archive.rmd160");
    for args in [
        vec!["genrsa", "-out", key.to_str().unwrap(), "2048"],
        vec![
            "rsa",
            "-in",
            key.to_str().unwrap(),
            "-pubout",
            "-out",
            public.to_str().unwrap(),
        ],
        vec![
            "dgst",
            "-ripemd160",
            "-sign",
            key.to_str().unwrap(),
            "-out",
            signature.to_str().unwrap(),
            staged.to_str().unwrap(),
        ],
    ] {
        let status = Command::new("/usr/bin/openssl")
            .args(&args)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{args:?}");
    }
    let url = serve_files(std::collections::HashMap::from([
        (format!("/lib/{filename}"), fs::read(&staged).unwrap()),
        (
            format!("/lib/{filename}.rmd160"),
            fs::read(&signature).unwrap(),
        ),
    ]));
    let prefix = tree.root.join("prefix");
    fs::write(
        tree.root.join("archive_sites.conf"),
        format!(
            "name legacy\nurls {url}\nprefix {0}\napplications_dir {0}/Applications\nframeworks_dir {0}/Library/Frameworks\ntype tbz2\n",
            prefix.display()
        ),
    )
    .unwrap();
    fs::write(
        tree.root.join("pubkeys.conf"),
        format!("{}\n", public.display()),
    )
    .unwrap();
    config(
        &tree,
        &format!(
            "archive_sites_conf {}",
            tree.root.join("archive_sites.conf").display()
        ),
    );
    config(
        &tree,
        &format!("pubkeys_conf {}", tree.root.join("pubkeys.conf").display()),
    );
    let installed = tree.run(&["install", "lib"]);
    let progress = String::from_utf8_lossy(&installed.stderr);
    assert!(installed.status.success(), "{progress}");
    assert!(
        progress.contains("Fetching archive for lib") && !progress.contains("Building lib"),
        "{progress}"
    );
    assert_eq!(read(&tree.file("share/lib/version")), "lib-1\n");
}

#[test]
fn non_flat_packages_are_bundles_with_bom_and_payload() {
    let tree = Tree::new();
    add_port(
        &tree,
        "app",
        &portfile("app", "1", "depends_lib port:lib")
            .replace("build {}", "package.flat no\nbuild {}"),
    );
    let lib = portfile("lib", "1", "").replace("build {}", "package.flat no\nbuild {}");
    fs::write(tree.root.join("ports/devel/lib/Portfile"), lib).unwrap();
    tree.ok(&["pkg", "app"]);
    let work = fs::read_dir(tree.root.join("db/build"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_devel_app"))
        .unwrap()
        .join("app/work");
    let contents = work.join("app-1.pkg/Contents");
    for file in [
        "Archive.bom",
        "Archive.pax.gz",
        "Info.plist",
        "PkgInfo",
        "Resources/Description.plist",
        "Resources/Archive.sizes",
    ] {
        assert!(contents.join(file).is_file(), "{file}");
    }
    let listing = Command::new("/usr/bin/lsbom")
        .arg("-s")
        .arg(contents.join("Archive.bom"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&listing.stdout).contains("share/app/version"));
    tree.ok(&["mpkg", "app"]);
    let packages = work.join("app-1.mpkg/Contents/Packages");
    assert!(packages.join("lib-1.pkg/Contents/Archive.bom").is_file());
    assert!(packages.join("app-1.pkg/Contents/Archive.bom").is_file());
    let info = read(&work.join("app-1.mpkg/Contents/Info.plist"));
    assert!(
        info.contains("<string>lib-1.pkg</string>") && info.contains("<string>app-1.pkg</string>"),
        "{info}"
    );
}

#[test]
fn ports_outside_the_tree_are_evaluated_from_their_directory() {
    let tree = Tree::new();
    let elsewhere = tree.root.join("elsewhere/ext");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("Portfile"), portfile("ext", "3", "")).unwrap();
    let directory = elsewhere.to_str().unwrap();
    assert_eq!(tree.ok(&["info", "--version", directory]), "version: 3\n");
    tree.ok(&["install", directory]);
    assert!(tree.ok(&["installed"]).contains("ext @3_0 (active)"));
    assert_eq!(read(&tree.file("share/ext/version")), "ext-3\n");
}
