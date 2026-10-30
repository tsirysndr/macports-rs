use port_rs::{progress::Progress, source};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
fn record() -> String {
    let body = "name curl version 8.0 description {Transfer URLs}";
    format!("curl {}\n{body}\n", body.len() + 1)
}
async fn serve(status: &str, body: String, extra_length: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/PortIndex", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len() + extra_length
    );
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let count = socket.read(&mut chunk).await.unwrap();
            if count == 0 {
                return;
            }
            request.extend_from_slice(&chunk[..count]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    url
}
#[tokio::test]
async fn initializes_valid_index_then_works_offline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("platform/PortIndex");
    let data = record();
    let url = serve("200 OK", data.clone(), 0).await;
    let index = source::initialize(&path, &url, &Progress::new(true))
        .await
        .unwrap();
    assert_eq!(index.ports.len(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
    let index = source::initialize(&path, "http://127.0.0.1:0/offline", &Progress::new(true))
        .await
        .unwrap();
    assert_eq!(index.ports["curl"].get("version"), "8.0");
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
}
#[tokio::test]
async fn rejects_invalid_empty_and_truncated_indexes_without_publishing() {
    for (data, extra) in [
        ("<html>error</html>".into(), 0),
        (String::new(), 0),
        (record(), 100),
        ("curl 500\nname curl\n".into(), 0),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PortIndex");
        let url = serve("200 OK", data, extra).await;
        assert!(
            source::initialize(&path, &url, &Progress::new(true))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
#[tokio::test]
async fn missing_platform_index_does_not_create_a_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PortIndex");
    let url = serve("404 Not Found", "missing".into(), 0).await;
    assert!(
        source::initialize(&path, &url, &Progress::new(true))
            .await
            .is_err()
    );
    assert!(!path.exists());
}
#[tokio::test]
async fn existing_corrupt_index_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PortIndex");
    std::fs::write(&path, "bad data").unwrap();
    assert!(
        source::initialize(&path, "http://127.0.0.1:0/offline", &Progress::new(true))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "bad data");
}
#[tokio::test]
async fn simultaneous_initializers_publish_one_complete_index() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PortIndex");
    let a = serve("200 OK", record(), 0).await;
    let b = serve("200 OK", record(), 0).await;
    let progress = Progress::new(true);
    let (a, b) = tokio::join!(
        source::initialize(&path, &a, &progress),
        source::initialize(&path, &b, &progress)
    );
    assert_eq!(a.unwrap().ports.len(), 1);
    assert_eq!(b.unwrap().ports.len(), 1);
    assert_eq!(std::fs::read_to_string(path).unwrap(), record());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
