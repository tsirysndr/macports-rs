use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session, TargetOutcome},
    progress::Progress,
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, path::Path};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const SHA256: &str = "41cf6794ba4200b839c53531555f0f3998df4cbb01a4d5cb0b94e3ca5e23947d";

async fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..count]);
                }
                let found = String::from_utf8_lossy(&request).starts_with("GET /ok/");
                let response = if found {
                    "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsource"
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            });
        }
    });
    url
}

fn session(distpath: &Path, sites: &str, checksums: &str) -> (Session, String) {
    let mut s = Session::open(
        &format!(
            r#"
            name app; version 1; license MIT
            master_sites {sites}
            distfiles app-1.tar.gz
            checksums {checksums}
            proc portfetch::get_full_mirror_sites_path {{}} {{return /nonexistent/mirrors.tcl}}
            proc portfetch::start_pings {{}} {{}}
            proc portfetch::check_dns {{}} {{}}
        "#
        ),
        &BTreeMap::from([
            ("distpath".into(), distpath.to_str().unwrap().into()),
            ("os.platform".into(), "darwin".into()),
            ("os.major".into(), "24".into()),
            (
                "portpath".into(),
                distpath.parent().unwrap().to_str().unwrap().into(),
            ),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    s.set_progress(Progress::new(true));
    let handle = s
        .attach_build_state(StateFile::open(&distpath.join("../state"), None).unwrap())
        .unwrap();
    (s, handle)
}

fn run(s: &mut Session, handle: &str, phase: &str) -> anyhow::Result<TargetOutcome> {
    let item = s.scalar(&format!("org.macports.{phase}")).unwrap();
    s.execute_target(&item, Some(handle), false)
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_and_checksum_targets_download_and_verify_distfiles() {
    let url = server().await;
    let root = tempfile::tempdir().unwrap();
    let distpath = root.path().join("distfiles");
    fs::create_dir(&distpath).unwrap();
    let (mut s, handle) = session(
        &distpath,
        &format!("{url}/missing/ {url}/ok/"),
        &format!("sha256 {SHA256} size 6"),
    );
    assert_eq!(
        run(&mut s, &handle, "fetch").unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        fs::read_to_string(distpath.join("app-1.tar.gz")).unwrap(),
        "source"
    );
    assert_eq!(
        run(&mut s, &handle, "checksum").unwrap(),
        TargetOutcome::Completed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_target_rejects_tampered_distfile_and_fetch_reports_mirrors() {
    let url = server().await;
    let root = tempfile::tempdir().unwrap();
    let distpath = root.path().join("distfiles");
    fs::create_dir(&distpath).unwrap();
    let (mut s, handle) = session(&distpath, &format!("{url}/missing/"), "size 6");
    let error = run(&mut s, &handle, "fetch").unwrap_err().to_string();
    assert!(error.contains("fetch of app-1.tar.gz failed"), "{error}");
    assert!(!distpath.join("app-1.tar.gz").exists());
    drop(s);
    fs::write(distpath.join("app-1.tar.gz"), "tampered").unwrap();
    let (mut s, handle) = session(&distpath, &format!("{url}/missing/"), "size 6");
    run(&mut s, &handle, "fetch").unwrap();
    let error = run(&mut s, &handle, "checksum").unwrap_err().to_string();
    assert!(error.contains("Unable to verify file checksums"), "{error}");
}

#[test]
fn execution_derives_upstream_build_and_work_paths() {
    let mut s = Session::open(
        "name app; version 1; subport app-extra {}",
        &BTreeMap::from([
            ("portdbpath".into(), "/db".into()),
            ("portpath".into(), "/ports/devel/app".into()),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    assert_eq!(
        s.scalar("portbuildpath").unwrap(),
        "/db/build/_ports_devel_app"
    );
    assert_eq!(
        s.scalar("workpath").unwrap(),
        "/db/build/_ports_devel_app/app/work"
    );
    assert_eq!(s.scalar("distpath").unwrap(), "/db/distfiles/app");
}
