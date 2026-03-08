#[path = "../src/error.rs"]
mod error;
#[path = "../src/api_client.rs"]
mod api_client;
mod common;

use api_client::ApiClient;
use common::run_with_client;
use mockito::Server;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn test_client_initialization() {
    // Client should initialize with a valid URL and runtime handle.
    let runtime = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    let client = ApiClient::new(
        "http://127.0.0.1:8080".to_string(),
        "jwt".to_string(),
        runtime.handle().clone(),
    );

    assert!(client.is_ok());
}

#[test]
fn test_client_connection_string() {
    // Validates base URL concatenation through a successful health check.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/health")
        .match_header("authorization", "Bearer test-token")
        .with_status(200)
        .create();

    let result = run_with_client(server.url(), |client| client.health_check());

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_client_reconnect_logic() {
    // Client should recover after a transient server-side failure.
    let mut server = Server::new();
    let first = server.mock("GET", "/health").with_status(503).expect(1).create();
    let second = server.mock("GET", "/health").with_status(200).expect(1).create();

    let base = server.url();
    let first_result = run_with_client(base.clone(), |client| client.health_check());
    assert!(first_result.is_err());

    let second_result = run_with_client(base, |client| client.health_check());
    assert!(second_result.is_ok());

    first.assert();
    second.assert();
}

#[test]
fn test_client_handles_closed_connection() {
    // A connection that closes after first success should return an error on next retry.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener bind must succeed");
    let addr = listener.local_addr().expect("local addr must exist");

    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length:0\r\n\r\n");
        }
    });

    let base = format!("http://{}:{}", addr.ip(), addr.port());

    let first = run_with_client(base.clone(), |client| client.health_check());
    assert!(first.is_ok());

    handle.join().expect("server thread must complete");

    let second = run_with_client(base, |client| client.health_check());
    assert!(second.is_err());
    assert_eq!(second.expect_err("must be error").errno, libc::EHOSTUNREACH);
}
