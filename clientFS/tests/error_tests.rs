#[path = "../src/error.rs"]
mod error;
#[path = "../src/api_client.rs"]
mod api_client;
mod common;

use common::run_with_client;
use mockito::Server;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn unused_local_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("port allocation must succeed");
    let port = listener.local_addr().expect("local addr must exist").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[test]
fn test_invalid_command_error() {
    // HTTP 405 must map to ENOSYS for unsupported operation.
    let mut server = Server::new();
    let mock = server.mock("GET", "/health").with_status(405).create();

    let err = run_with_client(server.url(), |client| {
        client.health_check().expect_err("405 should fail")
    });

    assert_eq!(err.errno, libc::ENOSYS);
    assert!(err.message.contains("Operation not supported"));
    mock.assert();
}

#[test]
fn test_invalid_response_format() {
    // Invalid JSON format must be surfaced as an I/O parse error.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/list/fmt")
        .with_status(200)
        .with_body("not-a-json")
        .create();

    let err = run_with_client(server.url(), |client| {
        client
            .list_directory("/fmt")
            .expect_err("invalid JSON should fail parsing")
    });

    assert_eq!(err.errno, libc::EIO);
    assert!(err.message.contains("Failed to parse response"));
    mock.assert();
}

#[test]
fn test_unexpected_server_message() {
    // JSON missing expected fields must fail deterministic deserialization.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/list/unexpected")
        .with_status(200)
        .with_body(r#"{"unexpected":[]}"#)
        .create();

    let err = run_with_client(server.url(), |client| {
        client
            .list_directory("/unexpected")
            .expect_err("unexpected schema should fail")
    });

    assert_eq!(err.errno, libc::EIO);
    assert!(err.message.contains("Failed to parse response"));
    mock.assert();
}

#[test]
fn test_timeout_error() {
    // A hanging HTTP endpoint must trigger the client's 5-second timeout mapping.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener bind must succeed");
    let addr = listener.local_addr().expect("local addr must exist");

    let server_thread = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 512];
            let _ = stream.read(&mut buf);
            thread::sleep(Duration::from_secs(6));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length:0\r\n\r\n");
        }
    });

    let base = format!("http://{}:{}", addr.ip(), addr.port());
    let err = run_with_client(base, |client| {
        client.health_check().expect_err("request should timeout")
    });

    assert_eq!(err.errno, libc::ETIMEDOUT);
    assert!(err.message.contains("timeout") || err.message.contains("Timeout"));

    server_thread.join().expect("server thread must finish");
}

#[test]
fn test_connection_refused_error() {
    // Connecting to a closed local port must map to connection error.
    let err = run_with_client(unused_local_url(), |client| {
        client
            .health_check()
            .expect_err("connection refused should fail")
    });

    assert_eq!(err.errno, libc::EHOSTUNREACH);
    assert!(err.message.contains("Cannot connect to server"));
}

#[test]
fn test_unauthorized_error() {
    // HTTP 401 must map to permission denied.
    let mut server = Server::new();
    let mock = server.mock("GET", "/health").with_status(401).create();

    let err = run_with_client(server.url(), |client| {
        client.health_check().expect_err("401 should fail")
    });

    assert_eq!(err.errno, libc::EACCES);
    assert!(err.message.contains("Permission denied"));
    mock.assert();
}

#[test]
fn test_forbidden_error() {
    // HTTP 403 must map to permission denied.
    let mut server = Server::new();
    let mock = server.mock("GET", "/health").with_status(403).create();

    let err = run_with_client(server.url(), |client| {
        client.health_check().expect_err("403 should fail")
    });

    assert_eq!(err.errno, libc::EACCES);
    assert!(err.message.contains("Permission denied"));
    mock.assert();
}

#[test]
fn test_internal_server_error() {
    // HTTP 500 must map to a generic I/O error.
    let mut server = Server::new();
    let mock = server.mock("GET", "/health").with_status(500).create();

    let err = run_with_client(server.url(), |client| {
        client.health_check().expect_err("500 should fail")
    });

    assert_eq!(err.errno, libc::EIO);
    assert!(err.message.contains("Internal server error"));
    mock.assert();
}
