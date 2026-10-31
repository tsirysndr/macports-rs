use port_rs::download::{self, Download, Manifest};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const BODY: &[u8] = b"a verified distfile\n";
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (r, a, p) = (requests.clone(), active.clone(), peak.clone());
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut socket, _) = connection.unwrap();
                        let (r,a,p) = (r.clone(),a.clone(),p.clone());
                        tasks.spawn(async move {
                            let mut request = Vec::new();
                            loop {
                                let mut bytes = [0;1024];
                                let n = socket.read(&mut bytes).await.unwrap();
                                if n == 0 { return; }
                                request.extend_from_slice(&bytes[..n]);
                                if request.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                            }
                            let path = String::from_utf8_lossy(&request).split_whitespace().nth(1).unwrap().to_owned();
                            let number = r.fetch_add(1,Ordering::SeqCst);
                            let current = a.fetch_add(1,Ordering::SeqCst) + 1;
                            p.fetch_max(current,Ordering::SeqCst);
                            if path == "/slow" { tokio::time::sleep(std::time::Duration::from_millis(80)).await; }
                            let status = if path == "/missing" { "404 Not Found" } else if path == "/retry" && number == 0 { "503 Unavailable" } else { "200 OK" };
                            let length = BODY.len() + if path == "/short" { 10 } else { 0 };
                            let header = if path == "/unknown" { format!("HTTP/1.1 {status}\r\nConnection: close\r\n\r\n") }
                                else { format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n") };
                            let _ = socket.write_all(header.as_bytes()).await;
                            let _ = socket.write_all(BODY).await;
                            let _ = socket.shutdown().await;
                            a.fetch_sub(1,Ordering::SeqCst);
                        });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Self {
            url,
            requests,
            peak,
            task,
        }
    }
    fn file(&self, name: &str, route: &str) -> Download {
        Download {
            name: name.into(),
            urls: vec![format!("{}{route}", self.url)],
            sha256: format!("{:x}", Sha256::digest(BODY)),
            size: Some(BODY.len() as u64),
        }
    }
}
#[tokio::test]
async fn downloads_are_concurrent_and_bounded() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        files: (0..6)
            .map(|i| server.file(&format!("{i}.tar"), "/slow"))
            .collect(),
    };
    let results = download::run(&manifest, dir.path(), 2).await.unwrap();
    assert_eq!(results.len(), 6);
    assert_eq!(server.peak.load(Ordering::SeqCst), 2);
    for result in results {
        assert_eq!(std::fs::read(result.path).unwrap(), BODY);
        assert!(!result.cached);
    }
}
#[tokio::test]
async fn verified_cache_avoids_network_and_corrupt_cache_is_preserved() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        files: vec![server.file("file", "/ok")],
    };
    download::run(&manifest, dir.path(), 1).await.unwrap();
    assert!(download::run(&manifest, dir.path(), 1).await.unwrap()[0].cached);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    std::fs::write(dir.path().join("file"), "corruption").unwrap();
    assert!(download::run(&manifest, dir.path(), 1).await.is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("file")).unwrap(),
        "corruption"
    );
}
#[tokio::test]
async fn checksum_failure_never_publishes_and_cleans_temporary_files() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut file = server.file("file", "/ok");
    file.sha256 = "0".repeat(64);
    let error = download::run(&Manifest { files: vec![file] }, dir.path(), 1)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("SHA-256 mismatch"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn retries_transient_failure() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    download::run(
        &Manifest {
            files: vec![server.file("file", "/retry")],
        },
        dir.path(),
        1,
    )
    .await
    .unwrap();
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn falls_back_to_next_mirror() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut file = server.file("file", "/missing");
    file.urls.push(format!("{}/ok", server.url));
    download::run(&Manifest { files: vec![file] }, dir.path(), 1)
        .await
        .unwrap();
}
#[tokio::test]
async fn truncated_body_is_not_published() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    assert!(
        download::run(
            &Manifest {
                files: vec![server.file("file", "/short")]
            },
            dir.path(),
            1
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn unknown_length_is_supported() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut file = server.file("file", "/unknown");
    file.size = None;
    download::run(&Manifest { files: vec![file] }, dir.path(), 1)
        .await
        .unwrap();
}
#[tokio::test]
async fn oversized_response_is_rejected() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut file = server.file("file", "/ok");
    file.size = Some(1);
    assert!(
        download::run(&Manifest { files: vec![file] }, dir.path(), 1)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn validates_entire_batch_before_writes() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    for invalid in ["../escape", "/absolute", ".", "", "a/b", "a\\b"] {
        assert!(
            download::run(
                &Manifest {
                    files: vec![server.file("good", "/ok"), server.file(invalid, "/ok")]
                },
                dir.path(),
                2
            )
            .await
            .is_err()
        );
    }
    let manifest = Manifest {
        files: vec![server.file("Same", "/ok"), server.file("same", "/ok")],
    };
    assert!(download::run(&manifest, dir.path(), 2).await.is_err());
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_destination_is_rejected() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), BODY).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("file")).unwrap();
    assert!(
        download::run(
            &Manifest {
                files: vec![server.file("file", "/ok")]
            },
            dir.path(),
            1
        )
        .await
        .is_err()
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}
