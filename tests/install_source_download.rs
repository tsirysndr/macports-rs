use port_rs::{
    install::source::{self, Options, Request},
    progress::Progress,
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn server(body: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
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
                let path = String::from_utf8_lossy(&request)
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let response = if path.starts_with("/ok/") {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .into()
                };
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            });
        }
    });
    (url, calls)
}

fn request(directory: &Path, urls: Vec<String>) -> Request {
    Request {
        filename: "app-1.tar.gz".into(),
        destination: directory.join("app-1.tar.gz"),
        sites: urls.clone(),
        urls,
    }
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn falls_back_across_mirrors_and_fetches_duplicates_once() {
    let (url, calls) = server("source").await;
    let directory = tempfile::tempdir().unwrap();
    let entry = request(
        directory.path(),
        vec![
            format!("{url}/missing/app-1.tar.gz"),
            format!("{url}/ok/app-1.tar.gz"),
        ],
    );
    let fetched = source::download(
        &[entry.clone(), entry],
        &Options::default(),
        &Progress::new(true),
        None,
    )
    .await
    .unwrap();
    assert_eq!(fetched, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(entries(directory.path()), ["app-1.tar.gz"]);
    assert_eq!(
        fs::read_to_string(directory.path().join("app-1.tar.gz")).unwrap(),
        "source"
    );
    let mode = fs::metadata(directory.path().join("app-1.tar.gz"))
        .unwrap()
        .permissions();
    assert_ne!(
        std::os::unix::fs::PermissionsExt::mode(&mode) & 0o044,
        0,
        "distfiles must not stay private to the fetching user"
    );
}

#[tokio::test]
async fn copies_file_urls_and_skips_existing_destinations() {
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("local.tar.gz"), "local").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file_url = format!("file://{}/local.tar.gz", input.path().display());
    let entry = request(directory.path(), vec![file_url]);
    assert_eq!(
        source::download(
            std::slice::from_ref(&entry),
            &Options::default(),
            &Progress::new(true),
            None
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(fs::read_to_string(&entry.destination).unwrap(), "local");
    let unreachable = request(directory.path(), vec!["http://127.0.0.1:9/x".into()]);
    assert_eq!(
        source::download(
            &[unreachable],
            &Options::default(),
            &Progress::new(true),
            None
        )
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn failure_reports_every_mirror_and_leaves_no_partial_file() {
    let (url, _) = server("source").await;
    let directory = tempfile::tempdir().unwrap();
    let entry = request(
        directory.path(),
        vec![
            format!("{url}/missing/app-1.tar.gz"),
            "ftp://mirror/app-1.tar.gz".into(),
        ],
    );
    let error = source::download(&[entry], &Options::default(), &Progress::new(true), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("fetch of app-1.tar.gz failed"), "{error}");
    assert!(error.contains("404"), "{error}");
    assert!(error.contains("unsupported URL scheme ftp"), "{error}");
    assert!(entries(directory.path()).is_empty());
    let empty = request(directory.path(), Vec::new());
    assert!(
        source::download(&[empty], &Options::default(), &Progress::new(true), None)
            .await
            .unwrap_err()
            .to_string()
            .contains("no fetch sites")
    );
}

#[tokio::test]
async fn sends_credentials_and_agent_and_applies_remote_time() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let count = socket.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..count]);
        }
        sender
            .send(String::from_utf8_lossy(&request).to_lowercase())
            .unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nLast-Modified: Tue, 15 Nov 1994 08:12:31 GMT\r\nConnection: close\r\n\r\nok",
            )
            .await
            .unwrap();
        socket.shutdown().await.ok();
    });
    let directory = tempfile::tempdir().unwrap();
    let entry = request(directory.path(), vec![format!("{url}/ok/app-1.tar.gz")]);
    let options = Options {
        credentials: Some(("user".into(), "secret".into())),
        user_agent: Some("custom-agent/1".into()),
        remote_time: true,
        ..Options::default()
    };
    source::download(
        std::slice::from_ref(&entry),
        &options,
        &Progress::new(true),
        None,
    )
    .await
    .unwrap();
    let headers = receiver.await.unwrap();
    assert!(
        headers.contains("authorization: basic dxnlcjpzzwnyzxq="),
        "{headers}"
    );
    assert!(headers.contains("user-agent: custom-agent/1"), "{headers}");
    let modified = fs::metadata(&entry.destination)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        784887151
    );
}
