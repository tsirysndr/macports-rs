#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::{
    index::Port,
    install::{Action, Item, binary::Installer, inspect::Limits, store::Store, target::Target},
    progress::Progress,
    registry::write::{NewImage, Writer},
    source::Platform,
};
use std::{collections::BTreeMap, fs};
fn port() -> Port {
    Port {
        name: "app".into(),
        fields: BTreeMap::from([
            ("version".into(), "1".into()),
            ("revision".into(), "0".into()),
        ]),
    }
}
fn item(port: &Port) -> Item<'_> {
    Item {
        port,
        variants: "+ssl".into(),
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    }
}
fn target() -> Target {
    let mut session = port_rs::portfile::Session::open(
        "name app; version 1; platforms darwin; set configure.build_arch arm64; set os.arch arm",
        &BTreeMap::new(),
        &port_rs::spec::PortSpec::named("app"),
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
fn record() -> NewImage {
    NewImage {
        name: "app".into(),
        version: "1".into(),
        variants: "+ssl".into(),
        archs: "arm64".into(),
        dependencies: vec!["base".into()],
        ..NewImage::default()
    }
}
fn archive() -> port_rs::install::archive::VerifiedArchive {
    let c = contents(&["opt/local/app", "opt/local/link"]);
    stage(&compress(&tar_with_owner(
        &[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("opt/local/app", b'0', &0xfeedfacfu32.to_ne_bytes(), ""),
            ("opt/local/link", b'2', b"", "app"),
        ],
        unsafe { libc::geteuid() }.into(),
        unsafe { libc::getegid() }.into(),
    )))
}
#[test]
fn installs_verified_binary_with_generated_inventory_and_requested_status() {
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    let path = database.path().join("registry.db");
    let store = Store::open(software.path()).unwrap();
    let mut registry = Writer::create(&path).unwrap();
    let progress = Progress::new(true);
    let port = port();
    let installed = Installer {
        store: &store,
        registry: &mut registry,
        staging: work.path(),
        root: root.path(),
        limits: Limits::default(),
        progress: &progress,
    }
    .install(&mut archive(), &item(&port), &target(), record())
    .unwrap();
    assert!(installed.location.is_file());
    assert_eq!(
        fs::read(root.path().join("opt/local/app")).unwrap(),
        0xfeedfacfu32.to_ne_bytes()
    );
    assert_eq!(
        fs::read_link(root.path().join("opt/local/link")).unwrap(),
        std::path::Path::new("app")
    );
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state,requested FROM ports WHERE id=?",
            [installed.id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("installed".into(), 1)
    );
    assert_eq!(
        db.query_row(
            "SELECT binary FROM files WHERE path='/opt/local/app'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT binary FROM files WHERE path='/opt/local/link'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn rejects_unresolved_plans_before_mutation_and_retains_registered_image_on_activation_conflict() {
    let software = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    let store = Store::open(software.path()).unwrap();
    let mut registry = Writer::create(&database.path().join("registry.db")).unwrap();
    let progress = Progress::new(true);
    let port = port();
    let mut installer = Installer {
        store: &store,
        registry: &mut registry,
        staging: work.path(),
        root: root.path(),
        limits: Limits::default(),
        progress: &progress,
    };
    for step in [Action::Activate, Action::AlreadyActive, Action::Resolve] {
        let mut item = item(&port);
        item.action = step;
        assert!(
            installer
                .install(&mut archive(), &item, &target(), record())
                .is_err()
        );
    }
    let mut unresolved = item(&port);
    unresolved
        .provider_fallbacks
        .push("path:lib/app:app".into());
    assert!(
        installer
            .install(&mut archive(), &unresolved, &target(), record())
            .is_err()
    );
    unresolved.provider_fallbacks.clear();
    unresolved.variants_resolved = false;
    assert!(
        installer
            .install(&mut archive(), &unresolved, &target(), record())
            .is_err()
    );
    assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let mut wrong = record();
    wrong.version = "2".into();
    assert!(
        installer
            .install(&mut archive(), &item(&port), &target(), wrong)
            .is_err()
    );
    assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    assert!(
        installer
            .registry
            .transaction()
            .unwrap()
            .entries()
            .unwrap()
            .is_empty()
    );
    fs::create_dir_all(root.path().join("opt/local")).unwrap();
    fs::write(root.path().join("opt/local/app"), b"existing").unwrap();
    let error = installer
        .install(&mut archive(), &item(&port), &target(), record())
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("retained for retry"));
    assert_eq!(
        fs::read(root.path().join("opt/local/app")).unwrap(),
        b"existing"
    );
    assert!(!root.path().join("opt/local/link").exists());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
    let entries = registry.transaction().unwrap().entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].active);
    assert_eq!(
        fs::read_dir(software.path().join("app")).unwrap().count(),
        2
    );
}

fn download_job<'a>(
    port: &'a Port,
    url: &str,
    dependencies: Vec<String>,
) -> port_rs::install::binary::Job<'a> {
    let source = format!(
        "name {}; version 1; revision 0; epoch 0; variant ssl {{}}; platforms darwin; set configure.build_arch arm64; set os.arch arm; set configure.cxx_stdlib libc++; set cxx_stdlib libc++",
        port.name
    );
    let mut requested = port_rs::spec::PortSpec::named(&port.name);
    requested.parse_modifier("+ssl").unwrap();
    let mut session = port_rs::portfile::Session::open(
        &source,
        &BTreeMap::new(),
        &requested,
        port_rs::portfile::PortGroups::default(),
    )
    .unwrap();
    let (target, registration) = port_rs::install::registration::from_session(
        &mut session,
        &item(port),
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        port_rs::install::registration::Inputs {
            requested: &requested,
            dependencies,
            portfile: source,
        },
    )
    .unwrap();
    port_rs::install::binary::Job {
        item: item(port),
        target,
        registration,
        request: port_rs::install::fetch::Request {
            name: port.name.clone(),
            urls: vec![format!("{url}/{}", port.name)],
            max_bytes: 1024 * 1024,
        },
    }
}

#[tokio::test]
async fn batch_downloads_concurrently_then_installs_dependencies_in_order() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for corrupt in [false, true] {
        let software = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        let store = Store::open(software.path()).unwrap();
        let mut registry = Writer::create(&database.path().join("registry.db")).unwrap();
        let mut base = port();
        base.name = "base".into();
        let app = port();
        let mut routes = std::collections::HashMap::new();
        for name in ["base", "app"] {
            let c = format!(
                "@name {name}-1_0+ssl\n@portname {name}\n@portversion 1\n@portrevision 0\n@portepoch 0\n@archs arm64\n@portvariant +ssl\nopt/local/{name}\n"
            );
            let bytes = compress(&tar_with_owner(
                &[
                    ("+CONTENTS", b'0', c.as_bytes(), ""),
                    (&format!("opt/local/{name}"), b'0', name.as_bytes(), ""),
                ],
                unsafe { libc::geteuid() }.into(),
                unsafe { libc::getegid() }.into(),
            ));
            let signed = stage(&bytes);
            routes.insert(
                format!("/{name}.sig"),
                signed.signature().as_bytes().to_vec(),
            );
            routes.insert(
                format!("/{name}"),
                if corrupt && name == "app" {
                    b"tampered".to_vec()
                } else {
                    bytes
                },
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (task_calls, task_peak) = (calls.clone(), peak.clone());
        let routes = Arc::new(routes);
        let server = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            while let Ok((mut socket, _)) = listener.accept().await {
                let (routes, calls, active, peak) = (
                    routes.clone(),
                    task_calls.clone(),
                    active.clone(),
                    task_peak.clone(),
                );
                handlers.spawn(async move {
                    let mut header = Vec::new();
                    loop {
                        let mut buffer = [0; 1024];
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        header.extend_from_slice(&buffer[..count]);
                        if header.windows(4).any(|part| part == b"\r\n\r\n") {
                            break;
                        }
                    }
                    calls.fetch_add(1, Ordering::SeqCst);
                    peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let text = String::from_utf8(header).unwrap();
                    let body = &routes[text.split_whitespace().nth(1).unwrap()];
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(body).await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        let progress = Progress::new(true);
        let keys = [trusted_key()];
        // Invalid order is rejected before the first HTTP request.
        assert!(
            port_rs::install::binary::fetch_batch(
                vec![
                    download_job(&app, &url, vec!["base".into()]),
                    download_job(&base, &url, vec![])
                ],
                work.path(),
                &keys,
                2,
                &progress
            )
            .await
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let batch = port_rs::install::binary::fetch_batch(
            vec![
                download_job(&base, &url, vec![]),
                download_job(&app, &url, vec!["base".into()]),
            ],
            work.path(),
            &keys,
            2,
            &progress,
        )
        .await;
        assert_eq!(fs::read_dir(software.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        assert!(
            registry
                .transaction()
                .unwrap()
                .entries()
                .unwrap()
                .is_empty()
        );
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        if corrupt {
            assert!(batch.is_err());
        } else {
            let installed = batch
                .unwrap()
                .install(&mut Installer {
                    store: &store,
                    registry: &mut registry,
                    staging: work.path(),
                    root: root.path(),
                    limits: Limits::default(),
                    progress: &progress,
                })
                .unwrap();
            assert_eq!(installed.len(), 2);
            assert!(installed[0].id < installed[1].id);
            assert_eq!(
                fs::read(root.path().join("opt/local/base")).unwrap(),
                b"base"
            );
            assert_eq!(fs::read(root.path().join("opt/local/app")).unwrap(), b"app");
            assert!(
                registry
                    .transaction()
                    .unwrap()
                    .entries()
                    .unwrap()
                    .iter()
                    .all(|entry| entry.active)
            );
        }
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
        server.abort();
    }
}
