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
        platform: "darwin".into(),
        os_major: "24".into(),
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
    let mut db = port_rs::registry::sqlite::Connection::open(&path).unwrap();
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
            (),
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT binary FROM files WHERE path='/opt/local/link'",
            (),
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
    assert!(
        format!("{error:#}").contains("retained for retry"),
        "{error:#}"
    );
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

fn signed_by(bytes: &[u8], seed: u8) -> (port_rs::install::signature::TrustedKey, Vec<u8>) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_compact::{KeyPair, Seed};
    let key = KeyPair::from_seed(Seed::new([seed; 32]));
    let envelope = |data: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(data);
        format!("untrusted comment: fixture\n{}\n", STANDARD.encode(packet))
    };
    (
        port_rs::install::signature::TrustedKey::parse(&envelope(key.pk.as_ref())).unwrap(),
        envelope(key.sk.sign(bytes, None).as_ref()).into_bytes(),
    )
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
    for (corrupt, isolated, wrong_key) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, true, true),
    ] {
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
            let signature = if isolated {
                signed_by(&bytes, if name == "base" { 11 } else { 22 }).1
            } else {
                stage(&bytes).signature().as_bytes().to_vec()
            };
            routes.insert(format!("/{name}.sig"), signature);
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
        let proxies = port_rs::install::proxy::Resolved {
            proxies: Default::default(),
            no_proxy: None,
        };
        let batch = if isolated {
            let trusted = |job, seed| port_rs::install::binary::TrustedJob {
                job,
                keys: vec![signed_by(b"", seed).0],
            };
            assert!(
                port_rs::install::binary::fetch_trusted_batch(
                    vec![
                        trusted(download_job(&app, &url, vec!["base".into()]), 22),
                        trusted(download_job(&base, &url, vec![]), 11),
                    ],
                    work.path(),
                    2,
                    &progress,
                    &proxies
                )
                .await
                .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            port_rs::install::binary::fetch_trusted_batch(
                vec![
                    trusted(download_job(&base, &url, vec![]), 11),
                    trusted(
                        download_job(&app, &url, vec!["base".into()]),
                        if wrong_key { 11 } else { 22 },
                    ),
                ],
                work.path(),
                2,
                &progress,
                &proxies,
            )
            .await
        } else {
            port_rs::install::binary::fetch_batch_with_proxies(
                vec![
                    download_job(&base, &url, vec![]),
                    download_job(&app, &url, vec!["base".into()]),
                ],
                work.path(),
                &keys,
                2,
                &progress,
                &proxies,
            )
            .await
        };
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
        if corrupt || wrong_key {
            assert!(batch.is_err());
        } else {
            let mut installer = Installer {
                store: &store,
                registry: &mut registry,
                staging: work.path(),
                root: root.path(),
                limits: Limits::default(),
                progress: &progress,
            };
            let runtime = tempfile::tempdir().unwrap();
            let installed = if isolated {
                use port_rs::{
                    portfile::{PortGroups, Session},
                    spec::PortSpec,
                };
                let mut sessions = Vec::new();
                for port in [&base, &app] {
                    let portpath = runtime.path().join(&port.name);
                    fs::create_dir(&portpath).unwrap();
                    let source = format!(
                        "{}; pre-install {{if {{$subport eq \"app\" && ![file exists $base_payload]}} {{error missing-dependency}}}}; post-activate {{notes \"$subport ready\"}}",
                        download_job(port, &url, vec![]).registration.portfile
                    );
                    fs::write(portpath.join("Portfile"), &source).unwrap();
                    let mut requested = PortSpec::named(&port.name);
                    requested.parse_modifier("+ssl").unwrap();
                    let vars = BTreeMap::from([
                        ("portpath".into(), portpath.to_str().unwrap().into()),
                        (
                            "portdbpath".into(),
                            database.path().to_str().unwrap().into(),
                        ),
                        (
                            "portbuildpath".into(),
                            portpath.join("build").to_str().unwrap().into(),
                        ),
                        (
                            "subbuildpath".into(),
                            portpath.join("build/port").to_str().unwrap().into(),
                        ),
                        (
                            "workpath".into(),
                            portpath.join("build/port/work").to_str().unwrap().into(),
                        ),
                        ("prefix_frozen".into(), root.path().to_str().unwrap().into()),
                        (
                            "configure.developer_dir".into(),
                            "/selected/Developer".into(),
                        ),
                        (
                            "base_payload".into(),
                            root.path().join("opt/local/base").to_str().unwrap().into(),
                        ),
                    ]);
                    let mut session =
                        Session::open(&source, &vars, &requested, PortGroups::default()).unwrap();
                    session
                        .enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
                        .unwrap();
                    sessions.push(session);
                }
                let results = batch
                    .unwrap()
                    .install_prepared(&mut installer, &mut sessions, |session, item| {
                        let mut requested = PortSpec::named(&item.port.name);
                        requested.parse_modifier("+ssl")?;
                        port_rs::install::registration::from_session(
                            session,
                            item,
                            &Platform {
                                darwin_major: 24,
                                architecture: "arm",
                            },
                            port_rs::install::registration::Inputs {
                                requested: &requested,
                                dependencies: if item.port.name == "app" {
                                    vec!["base".into()]
                                } else {
                                    vec![]
                                },
                                portfile: download_job(item.port, &url, vec![])
                                    .registration
                                    .portfile,
                            },
                        )
                    })
                    .unwrap();
                assert_eq!(
                    sessions[0].take_notifications(),
                    vec![("base".into(), "{base ready}".into())]
                );
                assert_eq!(
                    sessions[1].take_notifications(),
                    vec![("app".into(), "{app ready}".into())]
                );
                results
                    .into_iter()
                    .map(|r| r.image.unwrap())
                    .collect::<Vec<_>>()
            } else {
                batch.unwrap().install(&mut installer).unwrap()
            };
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

#[test]
fn registration_and_activation_run_in_separate_hook_boundaries() {
    use port_rs::{
        build::state::StateFile,
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    for failure_phase in ["none", "install", "activate"] {
        let fail_after_install = failure_phase == "install";
        let software = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runtime_work = runtime.path().join("build/app/work");
        let store = Store::open(software.path()).unwrap();
        let registry_path = database.path().join("registry.db");
        let mut registry = Writer::create(&registry_path).unwrap();
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
        let source = r#"
            name app; version 1; set events {}
            pre-install {
                if {![info exists target_state_fd]} {error "missing owned state"}
                if {$env(HOME) ne "$workpath/.home"} {error "wrong HOME"}
                if {$env(TMPDIR) ne "$workpath/.tmp"} {error "wrong TMPDIR"}
                if {$env(DEVELOPER_DIR) ne "/selected/Developer"} {error "wrong developer"}
                lappend events pre-install
            }
            post-install {
                if {[file exists $payload]} {error "activated too early"}
                lappend events post-install
                if {$fail_after_install} {error "post-install failure"}
            }
            pre-activate {
                if {[file exists $payload]} {error "activated before pre-hook"}
                lappend events pre-activate
            }
            post-activate {
                if {![file exists $payload]} {error "payload not activated"}
                lappend events post-activate
                notes "app is ready"
                if {$fail_after_activate} {error "post-activate failure"}
            }
        "#;
        fs::write(runtime.path().join("Portfile"), source).unwrap();
        let mut session = Session::open(
            source,
            &BTreeMap::from([
                ("portpath".into(), runtime.path().to_str().unwrap().into()),
                (
                    "portdbpath".into(),
                    database.path().to_str().unwrap().into(),
                ),
                (
                    "portbuildpath".into(),
                    runtime.path().join("build").to_str().unwrap().into(),
                ),
                (
                    "subbuildpath".into(),
                    runtime.path().join("build/app").to_str().unwrap().into(),
                ),
                ("workpath".into(), runtime_work.to_str().unwrap().into()),
                (
                    "configure.developer_dir".into(),
                    "/selected/Developer".into(),
                ),
                (
                    "payload".into(),
                    root.path().join("opt/local/app").to_str().unwrap().into(),
                ),
                (
                    "fail_after_activate".into(),
                    if failure_phase == "activate" {
                        "1"
                    } else {
                        "0"
                    }
                    .into(),
                ),
                (
                    "fail_after_install".into(),
                    if fail_after_install { "1" } else { "0" }.into(),
                ),
            ]),
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        session.set_progress(Progress::new(true));
        session
            .invoke("set", &["prefix_frozen", root.path().to_str().unwrap()])
            .unwrap();
        session
            .enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
            .unwrap();
        let result = installer.install_prepared(
            &mut archive(),
            &item(&port),
            &mut session,
            false,
            |session| {
                assert_eq!(session.scalar("events")?, "pre-install");
                Ok((target(), record()))
            },
        );
        // The prepared runner owns the state lock across hooks and releases it
        // even when post-install fails after committing an inactive image.
        let mut state = StateFile::try_open(&runtime_work.join(".macports.app.state"), None)
            .unwrap()
            .expect("prepared target released its state lock");
        // Install is an always-run target: neither outcome records completion.
        assert!(!state.contains("target", "org.macports.install").unwrap());
        drop(state);
        for key in ["target_state_fd", "env(TMPDIR)", "env(DEVELOPER_DIR)"] {
            assert_eq!(session.invoke("info", &["exists", key]).unwrap(), "0");
        }
        let mut db = port_rs::registry::sqlite::Connection::open(&registry_path).unwrap();
        if fail_after_install {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("post-install failure")
            );
            assert_eq!(
                db.query_row("SELECT state FROM ports", (), |r| r.get::<_, String>(0))
                    .unwrap(),
                "imaged"
            );
            let location: String = db
                .query_row("SELECT location FROM ports", (), |r| r.get(0))
                .unwrap();
            assert!(std::path::Path::new(&location).is_file());
            assert!(!root.path().join("opt/local/app").exists());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
            assert_eq!(
                session.scalar("events").unwrap(),
                "pre-install post-install"
            );
            let mut activate_item = item(&port);
            activate_item.action = Action::Activate;
            let entries = installer.registry.transaction().unwrap().entries().unwrap();
            let image_id =
                port_rs::install::binary::select_inactive(&entries, &activate_item, &target())
                    .unwrap()
                    .id;
            let mut cached = store
                .load(
                    &activate_item,
                    &target(),
                    &[trusted_key()],
                    work.path(),
                    port_rs::install::store::LoadLimits {
                        archive_bytes: 1024 * 1024,
                        inspection: Limits::default(),
                    },
                )
                .unwrap();
            // A dry run must neither extract nor invoke hooks/target resolution.
            assert_eq!(
                installer
                    .activate_prepared(
                        &mut cached,
                        &activate_item,
                        image_id,
                        &mut session,
                        true,
                        |_| panic!("dry-run target resolution")
                    )
                    .unwrap(),
                port_rs::portfile::TargetOutcome::DryRun
            );
            assert!(!root.path().join("opt/local/app").exists());
            installer
                .activate_prepared(
                    &mut cached,
                    &activate_item,
                    image_id,
                    &mut session,
                    false,
                    |_| Ok(target()),
                )
                .unwrap();
            assert!(root.path().join("opt/local/app").is_file());
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM ports", (), |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                db.query_row("SELECT state FROM ports", (), |r| r.get::<_, String>(0))
                    .unwrap(),
                "installed"
            );
            assert_eq!(
                session.scalar("events").unwrap(),
                "pre-install post-install pre-activate post-activate"
            );
            assert_eq!(
                session.take_notifications(),
                vec![("app".into(), "{app is ready}".into())]
            );
            drop(cached);
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
        } else if failure_phase == "activate" {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("post-activate failure")
            );
            assert_eq!(
                db.query_row("SELECT state FROM ports", (), |r| r.get::<_, String>(0))
                    .unwrap(),
                "installed"
            );
            assert!(root.path().join("opt/local/app").is_file());
            let location: String = db
                .query_row("SELECT location FROM ports", (), |r| r.get(0))
                .unwrap();
            assert!(std::path::Path::new(&location).is_file());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
            assert_eq!(
                session.scalar("events").unwrap(),
                "pre-install post-install pre-activate post-activate"
            );
            assert!(session.take_notifications().is_empty());
        } else {
            let outcome = result.unwrap();
            assert_eq!(outcome.install, port_rs::portfile::TargetOutcome::Completed);
            assert_eq!(
                outcome.activate,
                Some(port_rs::portfile::TargetOutcome::Completed)
            );
            let location = outcome.image.unwrap().location;
            assert_eq!(
                db.query_row("SELECT state FROM ports", (), |r| r.get::<_, String>(0))
                    .unwrap(),
                "installed"
            );
            assert_eq!(
                session.scalar("events").unwrap(),
                "pre-install post-install pre-activate post-activate"
            );
            assert!(location.is_file());
            assert_eq!(
                session.take_notifications(),
                vec![("app".into(), "{app is ready}".into())]
            );
        }
    }
}

#[test]
fn prepared_install_dry_run_and_custom_targets_skip_native_registration() {
    use port_rs::{
        portfile::{PortGroups, Session, TargetOutcome},
        spec::PortSpec,
    };
    for dry_run in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let source = "name app; version 1; set events {}; install {lappend events custom-install}; activate {lappend events custom-activate}";
        fs::write(temp.path().join("Portfile"), source).unwrap();
        let path = |suffix: &str| temp.path().join(suffix).to_str().unwrap().to_owned();
        let mut session = Session::open(
            source,
            &BTreeMap::from([
                ("portpath".into(), path("")),
                ("portdbpath".into(), path("db")),
                ("portbuildpath".into(), path("build")),
                ("subbuildpath".into(), path("build/app")),
                ("workpath".into(), path("build/app/work")),
                ("prefix_frozen".into(), path("")),
                (
                    "configure.developer_dir".into(),
                    "/selected/Developer".into(),
                ),
            ]),
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        session
            .enable_system(&BTreeMap::new(), |_| panic!("no process expected"))
            .unwrap();
        let software = tempfile::tempdir().unwrap();
        let store = Store::open(software.path()).unwrap();
        let staging = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("registry.db");
        let mut registry = Writer::create(&db_path).unwrap();
        let progress = Progress::new(true);
        let mut installer = Installer {
            store: &store,
            registry: &mut registry,
            staging: staging.path(),
            root: root.path(),
            limits: Limits::default(),
            progress: &progress,
        };
        let outcome = installer
            .install_prepared(
                &mut archive(),
                &item(&port()),
                &mut session,
                dry_run,
                |_| panic!("native registration must not run"),
            )
            .unwrap();
        let expected = if dry_run {
            TargetOutcome::DryRun
        } else {
            TargetOutcome::Completed
        };
        assert_eq!(outcome.install, expected);
        assert_eq!(outcome.activate, Some(expected));
        assert!(outcome.image.is_none());
        assert_eq!(
            session.scalar("events").unwrap(),
            if dry_run {
                ""
            } else {
                "custom-install custom-activate"
            }
        );
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        let mut db = port_rs::registry::sqlite::Connection::open(db_path).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM ports", (), |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        if dry_run {
            assert!(!temp.path().join("build").exists());
        }
    }
}

#[test]
fn accepted_archive_marks_source_phases_only_after_live_identity_checks() {
    use port_rs::{
        build::state::StateFile,
        portfile::{PortGroups, Session, TargetOutcome},
        spec::PortSpec,
    };
    let work = tempfile::tempdir().unwrap();
    let state_path = work.path().join("state");
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("+ssl").unwrap();
    let mut s = Session::open(
        r#"
        name app; version 1; variant ssl {}
        set os.platform darwin; set os.subplatform macosx; set os.major 24
        set os.arch arm; set configure.build_arch arm64
        build {error source-body-must-not-run}
    "#,
        &BTreeMap::new(),
        &spec,
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&state_path, None).unwrap())
        .unwrap();
    s.invoke("set", &["target_state_fd", &fd]).unwrap();
    let mut bytes = archive();
    let port = port();
    for (key, value, original) in [
        ("version", "wrong", "1"),
        ("portvariants", "+other", "+ssl"),
        ("configure.build_arch", "x86_64", "arm64"),
        ("ports_source_only", "yes", "no"),
        ("ports_dryrun", "yes", "no"),
    ] {
        s.invoke("set", &[key, value]).unwrap();
        assert!(
            s.accept_verified_archive(&mut bytes, &item(&port), &target(), Limits::default())
                .is_err(),
            "{key}"
        );
        assert_eq!(
            s.invoke(
                "check_statefile",
                &["target", "org.macports.archivefetch", &fd]
            )
            .unwrap(),
            "0"
        );
        s.invoke("set", &[key, original]).unwrap();
    }
    for _ in 0..2 {
        s.accept_verified_archive(&mut bytes, &item(&port), &target(), Limits::default())
            .unwrap();
    }
    for phase in [
        "archivefetch",
        "fetch",
        "checksum",
        "extract",
        "patch",
        "configure",
        "build",
        "destroot",
    ] {
        assert_eq!(
            s.invoke(
                "check_statefile",
                &["target", &format!("org.macports.{phase}"), &fd]
            )
            .unwrap(),
            "1"
        );
    }
    for phase in ["install", "activate"] {
        assert_eq!(
            s.invoke(
                "check_statefile",
                &["target", &format!("org.macports.{phase}"), &fd]
            )
            .unwrap(),
            "0"
        );
    }
    let build = s.scalar("org.macports.build").unwrap();
    assert_eq!(
        s.execute_target(&build, Some(&fd), false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
    s.close_build_state(&fd).unwrap();
    let text = fs::read_to_string(state_path).unwrap();
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("target:"))
            .count(),
        8
    );
}
