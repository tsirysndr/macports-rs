use port_rs::{
    config::{Config, Paths},
    install::{
        Action, Item,
        sites::Layout,
        target::{Architecture, Target},
    },
    portfile::{PortGroups, Session},
    source::Platform,
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn session() -> Session {
    Session::open(
        "name curl; version 1",
        &BTreeMap::from([
            ("os.platform".into(), "darwin".into()),
            ("os.major".into(), "24".into()),
        ]),
        &PortSpec::named("curl"),
        PortGroups::default(),
    )
    .unwrap()
}
fn layout(paths: &Paths) -> Layout {
    Layout {
        prefix: paths.normalize("/opt/local").unwrap(),
        applications_dir: paths.normalize("/Applications/MacPorts").unwrap(),
        frameworks_dir: paths.normalize("/opt/local/Library/Frameworks").unwrap(),
        cxx_stdlib: "libc++".into(),
        delete_la_files: "yes".into(),
    }
}

#[test]
fn discovery_uses_active_tree_and_live_resource_path_overrides() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths {
        configuration_directory: root.path().join("etc"),
        working_directory: root.path().to_owned(),
        user_home: None,
        portsrc: None,
    };
    let resource_dir = root.path().join("tree/_resources/custom");
    std::fs::create_dir_all(&resource_dir).unwrap();
    std::fs::write(
        resource_dir.join("mirrors.tcl"),
        include_str!("fixtures/archive-sites/archive_sites.tcl"),
    )
    .unwrap();
    let mut session = session();
    let config = Config::default();
    assert!(
        paths
            .select_archive_sites(
                &config,
                std::path::Path::new("tree"),
                &mut session,
                &layout(&paths),
                |_| true
            )
            .unwrap()
            .defaults
            .is_empty()
    );
    session
        .invoke("archive_sites.listpath", &["custom"])
        .unwrap();
    session
        .invoke("archive_sites.listfile", &["mirrors.tcl"])
        .unwrap();
    let selected = paths
        .select_archive_sites(
            &config,
            std::path::Path::new("tree"),
            &mut session,
            &layout(&paths),
            |kind| kind == "tbz2",
        )
        .unwrap();
    assert_eq!(selected.defaults, ["macports_archives::tbz2"]);
    assert!(
        paths
            .select_archive_sites(
                &config,
                std::path::Path::new("other-tree"),
                &mut session,
                &layout(&paths),
                |_| true
            )
            .unwrap()
            .defaults
            .is_empty()
    );
    std::fs::create_dir_all(&paths.configuration_directory).unwrap();
    std::fs::write(
        paths.configuration_directory.join("archive_sites.conf"),
        "name custom\nurls https://custom/\n",
    )
    .unwrap();
    let selected = paths
        .select_archive_sites(
            &config,
            std::path::Path::new("other-tree"),
            &mut session,
            &layout(&paths),
            |_| true,
        )
        .unwrap();
    assert_eq!(selected.defaults, ["custom::tbz2"]);
}

#[tokio::test]
async fn discovered_site_request_and_keys_authenticate_a_download() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_compact::{KeyPair, Seed};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root = tempfile::tempdir().unwrap();
    let paths = Paths {
        configuration_directory: root.path().join("etc"),
        working_directory: root.path().to_owned(),
        user_home: None,
        portsrc: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/:nosubdir", listener.local_addr().unwrap());
    let bytes = b"authenticated archive transport fixture";
    let key = KeyPair::from_seed(Seed::new([67; 32]));
    let envelope = |bytes: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(bytes);
        format!("untrusted comment: fixture\n{}\n", STANDARD.encode(packet))
    };
    std::fs::write(root.path().join("site.pub"), envelope(key.pk.as_ref())).unwrap();
    let signature = envelope(key.sk.sign(bytes, None).as_ref());
    let resource_dir = root.path().join("tree/_resources/port1.0/fetch");
    std::fs::create_dir_all(&resource_dir).unwrap();
    let source = format!(
        "{}\nset ::portfetch::mirror_sites::sites(macports_archives) {{{base}}}\nset ::portfetch::mirror_sites::archive_pubkey(macports_archives) site.pub",
        include_str!("fixtures/archive-sites/archive_sites.tcl")
    );
    std::fs::write(resource_dir.join("archive_sites.tcl"), source).unwrap();
    let mut session = session();
    let selected = paths
        .select_archive_sites(
            &Config::default(),
            std::path::Path::new("tree"),
            &mut session,
            &layout(&paths),
            |kind| kind == "tbz2",
        )
        .unwrap();
    let port = session.metadata().port.clone();
    let item = Item {
        port: &port,
        variants: "".into(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    };
    let target = Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        "darwin",
        Architecture::Native,
    )
    .unwrap();
    let prepared = paths
        .prepare_archive_download(
            &Config::default(),
            &selected,
            &mut session,
            &target,
            &item,
            1024,
        )
        .unwrap();
    assert!(prepared.warnings.is_empty());
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let byte = stream.read_u8().await.unwrap();
                request.push(byte);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            assert!(path.starts_with("/curl/curl-1_0.darwin_24.arm64.tbz2"));
            let body = if path.ends_with(".sig") {
                signature.as_bytes()
            } else {
                bytes.as_slice()
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        }
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        port_rs::install::fetch::run_trusted(
            &[prepared.request],
            root.path(),
            2,
            &port_rs::progress::Progress::new(true),
            &port_rs::install::proxy::Resolved {
                proxies: BTreeMap::new(),
                no_proxy: None,
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].verification().bytes, bytes.len() as u64);
    server.await.unwrap();
}
