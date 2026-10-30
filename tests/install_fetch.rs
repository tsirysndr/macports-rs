use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_compact::{KeyPair, Seed};
use port_rs::{
    install::{
        fetch::{self, Request},
        signature::TrustedKey,
    },
    progress::Progress,
};
use std::{
    collections::HashMap,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn signed(bytes: &[u8]) -> (TrustedKey, Vec<u8>) {
    let key = KeyPair::from_seed(Seed::new([12; 32]));
    let envelope = |data: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(data);
        format!("untrusted comment: test\n{}\n", STANDARD.encode(packet))
    };
    (
        TrustedKey::parse(&envelope(key.pk.as_ref())).unwrap(),
        envelope(key.sk.sign(bytes, None).as_ref()).into_bytes(),
    )
}
struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
    peak: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(routes: Vec<(&str, Vec<u8>)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let routes: HashMap<String, Vec<u8>> = routes
        .into_iter()
        .map(|(path, body)| (path.into(), body))
        .collect();
    let routes = Arc::new(routes);
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (peak_task, calls_task) = (peak.clone(), calls.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (routes, active, peak, calls) = (
                routes.clone(),
                active.clone(),
                peak_task.clone(),
                calls_task.clone(),
            );
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                calls.fetch_add(1, Ordering::SeqCst);
                let concurrent = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(concurrent, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let response = routes.get(path).cloned().unwrap_or_else(|| {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_vec()
                });
                let _ = socket.write_all(&response).await;
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Server {
        url,
        task,
        peak,
        calls,
    }
}
fn response(bytes: &[u8]) -> Vec<u8> {
    let mut result = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .into_bytes();
    result.extend(bytes);
    result
}
fn chunked(bytes: &[u8]) -> Vec<u8> {
    let mut result =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    result.extend(format!("{:x}\r\n", bytes.len()).bytes());
    result.extend(bytes);
    result.extend(b"\r\n0\r\n\r\n");
    result
}
fn request(server: &Server, paths: &[&str], limit: u64) -> Request {
    Request {
        name: "app".into(),
        urls: paths
            .iter()
            .map(|path| format!("{}{path}", server.url))
            .collect(),
        max_bytes: limit,
    }
}
#[tokio::test]
async fn downloads_in_parallel_preserving_order_and_private_bytes() {
    let (key, sig_a) = signed(b"first");
    let (_, sig_b) = signed(b"second");
    let server = server(vec![
        ("/a", response(b"first")),
        ("/a.sig", response(&sig_a)),
        ("/b", response(b"second")),
        ("/b.sig", response(&sig_b)),
    ])
    .await;
    let temp = tempfile::tempdir().unwrap();
    let requests = [request(&server, &["/a"], 5), request(&server, &["/b"], 6)];
    let mut archives = fetch::run(&requests, temp.path(), &[key], 2, &Progress::new(true))
        .await
        .unwrap();
    for (archive, expected) in archives.iter_mut().zip([b"first".as_slice(), b"second"]) {
        let mut bytes = Vec::new();
        archive.reader().unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        assert_eq!(archive.verification().bytes, expected.len() as u64);
    }
    assert_eq!(server.peak.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn fallback_rejects_bad_signatures_before_accepting_next_mirror() {
    let (key, signature) = signed(b"authentic");
    let server = server(vec![
        ("/bad", response(b"tampered!")),
        ("/bad.sig", response(&signature)),
        ("/good", response(b"authentic")),
        ("/good.sig", response(&signature)),
    ])
    .await;
    let temp = tempfile::tempdir().unwrap();
    let requests = [request(&server, &["/missing", "/bad", "/good"], 100)];
    let result = fetch::run(&requests, temp.path(), &[key], 1, &Progress::new(true))
        .await
        .unwrap();
    assert_eq!(result[0].verification().bytes, 9);
    assert_eq!(server.calls.load(Ordering::SeqCst), 5);
}
#[tokio::test]
async fn bounds_chunked_bodies_and_signature_envelopes_without_publication() {
    let (key, signature) = signed(b"12345");
    let server = server(vec![
        ("/big", chunked(b"12345")),
        ("/big.sig", response(&signature)),
        ("/sig.sig", chunked(&vec![b'x'; 4097])),
    ])
    .await;
    let temp = tempfile::tempdir().unwrap();
    for path in ["/big", "/sig"] {
        let result = fetch::run(
            &[request(&server, &[path], 4)],
            temp.path(),
            std::slice::from_ref(&key),
            1,
            &Progress::new(true),
        )
        .await;
        assert!(result.err().unwrap().to_string().contains("size limit"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
#[tokio::test]
async fn failures_drain_batch_and_remove_successful_stages() {
    let (key, signature) = signed(b"good");
    let server = server(vec![
        ("/good", response(b"good")),
        ("/good.sig", response(&signature)),
    ])
    .await;
    let temp = tempfile::tempdir().unwrap();
    let requests = [
        request(&server, &["/missing"], 10),
        request(&server, &["/good"], 10),
    ];
    assert!(
        fetch::run(&requests, temp.path(), &[key], 1, &Progress::new(true))
            .await
            .is_err()
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 3);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn preflight_validates_whole_batch_before_network_requests() {
    let (key, _) = signed(b"");
    let server = server(vec![]).await;
    let temp = tempfile::tempdir().unwrap();
    for invalid in [
        "file:///tmp/archive",
        "https://host/archive?query",
        "https://user:pass@host/archive",
        "https://host/archive#fragment",
    ] {
        let mut bad = request(&server, &["/bad"], 10);
        bad.urls = vec![invalid.into()];
        let requests = [request(&server, &["/good"], 10), bad];
        assert!(
            fetch::run(
                &requests,
                temp.path(),
                std::slice::from_ref(&key),
                2,
                &Progress::new(true)
            )
            .await
            .is_err()
        );
    }
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn rejects_partial_and_truncated_http_responses() {
    let (key, signature) = signed(b"good");
    let server = server(vec![
        (
            "/partial.sig",
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
        ),
        ("/short.sig", response(&signature)),
        (
            "/short",
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\ngood".to_vec(),
        ),
    ])
    .await;
    let temp = tempfile::tempdir().unwrap();
    for path in ["/partial", "/short"] {
        assert!(
            fetch::run(
                &[request(&server, &[path], 1000)],
                temp.path(),
                std::slice::from_ref(&key),
                1,
                &Progress::new(true)
            )
            .await
            .is_err()
        );
    }
}
