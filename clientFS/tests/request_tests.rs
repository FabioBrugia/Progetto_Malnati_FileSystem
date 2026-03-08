#[path = "../src/error.rs"]
mod error;
#[path = "../src/api_client.rs"]
mod api_client;
#[path = "../src/auth.rs"]
mod auth;
mod common;

use common::run_with_client;
use mockito::{Matcher, Server};

#[test]
fn test_build_create_request() {
    // Validates POST /mkdir/{path} request path and auth header.
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/mkdir/dir/new")
        .match_header("authorization", "Bearer test-token")
        .with_status(201)
        .create();

    let result = run_with_client(server.url(), |client| client.create_directory("/dir/new"));

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_build_read_request() {
    // Validates GET /files/{path} with normalized path and body retrieval.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/files/docs/readme.txt")
        .match_header("authorization", "Bearer test-token")
        .with_status(200)
        .with_body("hello")
        .create();

    let result = run_with_client(server.url(), |client| {
        client.read_file("/docs/readme.txt").expect("read must succeed")
    });

    assert_eq!(result, b"hello");
    mock.assert();
}

#[test]
fn test_build_write_request() {
    // Validates PUT /files/{path} and payload encoding.
    let payload = b"payload-data".to_vec();
    let mut server = Server::new();
    let mock = server
        .mock("PUT", "/files/docs/out.txt")
        .match_header("authorization", "Bearer test-token")
        .match_body(Matcher::Exact("payload-data".to_string()))
        .with_status(200)
        .create();

    let result = run_with_client(server.url(), move |client| {
        client.write_file("/docs/out.txt", &payload)
    });

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_build_delete_request() {
    // Validates DELETE /files/{path} request format.
    let mut server = Server::new();
    let mock = server
        .mock("DELETE", "/files/tmp/old.txt")
        .match_header("authorization", "Bearer test-token")
        .with_status(204)
        .create();

    let result = run_with_client(server.url(), |client| client.delete("/tmp/old.txt"));

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_build_list_request() {
    // Validates GET /list/{path} and JSON list parsing.
    let mut server = Server::new();
    let body = r#"{"entries":[{"name":"a.txt","is_dir":false,"size":1,"mtime":1.0,"ctime":1.0,"mode":420}]}"#;
    let mock = server
        .mock("GET", "/list/projects")
        .match_header("authorization", "Bearer test-token")
        .with_status(200)
        .with_body(body)
        .create();

    let entries = run_with_client(server.url(), |client| {
        client
            .list_directory("/projects")
            .expect("list_directory must succeed")
    });

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a.txt");
    mock.assert();
}

#[test]
fn test_build_auth_request() {
    // Validates POST /auth JSON request payload and token extraction.
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/auth")
        .match_header("content-type", Matcher::Regex("application/json.*".to_string()))
        .match_body(Matcher::PartialJson(serde_json::json!({ "password": "secret" })))
        .with_status(200)
        .with_body(r#"{"token":"jwt-token"}"#)
        .create();

    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    let token = rt
        .block_on(auth::authenticate(&server.url(), "secret"))
        .expect("authenticate must succeed");

    assert_eq!(token, "jwt-token");
    mock.assert();
}

#[test]
fn test_empty_filename() {
    // Edge case: empty logical file name still maps to /files/ endpoint.
    let mut server = Server::new();
    let mock = server
        .mock("PUT", "/files/")
        .match_header("authorization", "Bearer test-token")
        .with_status(200)
        .create();

    let result = run_with_client(server.url(), |client| client.write_file("", b""));

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_long_filename() {
    // Edge case: very long path is preserved in request formatting.
    let long_name = format!("/{}", "a".repeat(180));
    let expected_path = format!("/files/{}", "a".repeat(180));

    let mut server = Server::new();
    let mock = server
        .mock("PUT", expected_path.as_str())
        .match_header("authorization", "Bearer test-token")
        .with_status(200)
        .create();

    let result = run_with_client(server.url(), move |client| client.write_file(&long_name, b"x"));

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_invalid_path() {
    // Edge case: malformed URL path must return an I/O class client error.
    let err = run_with_client("http://127.0.0.1:9999".to_string(), |client| {
        client
            .read_file("/bad\npath")
            .expect_err("invalid path should fail before network I/O")
    });

    assert!(
        err.errno == libc::EIO || err.errno == libc::EHOSTUNREACH,
        "unexpected errno for invalid path: {}",
        err.errno
    );
    assert!(err.message.contains("read_file"));
}

#[test]
fn test_large_payload_handling() {
    // Edge case: large payload should be sent without truncation.
    let payload = "x".repeat(256 * 1024);
    let mut server = Server::new();
    let mock = server
        .mock("PUT", "/files/big.bin")
        .match_header("authorization", "Bearer test-token")
        .match_body(Matcher::Exact(payload.clone()))
        .with_status(200)
        .create();

    let result = run_with_client(server.url(), move |client| {
        client.write_file("/big.bin", payload.as_bytes())
    });

    assert!(result.is_ok());
    mock.assert();
}
