use port_rs::install::mirror_probe::time_connect;
use tokio::{io::AsyncReadExt, net::TcpListener};
#[tokio::test]
async fn measures_tcp_connection_without_sending_http_or_other_payload() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut payload = Vec::new();
        socket.read_to_end(&mut payload).await.unwrap();
        assert!(payload.is_empty());
    });
    let elapsed = time_connect("127.0.0.1", &port).await.unwrap();
    assert!(elapsed.is_finite() && elapsed >= 0.0);
    server.await.unwrap();
}
#[tokio::test]
async fn refused_connections_and_invalid_services_are_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    drop(listener);
    assert!(
        time_connect("127.0.0.1", &port)
            .await
            .unwrap_err()
            .to_string()
            .contains("connection failed")
    );
    assert!(
        time_connect("127.0.0.1", "not-a-real-service")
            .await
            .is_err()
    );
    assert!(time_connect("bad\0host", "http").await.is_err());
}
