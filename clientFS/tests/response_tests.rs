#[path = "../src/error.rs"]
mod error;
#[path = "../src/api_client.rs"]
mod api_client;
mod common;

use common::run_with_client;
use mockito::Server;

#[test]
fn test_parse_ok_response() {
    // Parses a successful directory listing response.
    let mut server = Server::new();
    let body = r#"{"entries":[{"name":"notes.txt","is_dir":false,"size":12,"mtime":12.0,"ctime":10.0,"mode":420}]}"#;
    let mock = server.mock("GET", "/list/docs").with_status(200).with_body(body).create();

    let entries = run_with_client(server.url(), |client| {
        client.list_directory("/docs").expect("list should parse")
    });

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "notes.txt");
    assert_eq!(entries[0].size, 12);
    mock.assert();
}

#[test]
fn test_parse_error_response() {
    // HTTP error status must map to ApiError.
    let mut server = Server::new();
    let mock = server.mock("GET", "/files/missing.txt").with_status(404).create();

    let err = run_with_client(server.url(), |client| {
        client
            .read_file("/missing.txt")
            .expect_err("404 must become ENOENT")
    });

    assert_eq!(err.errno, libc::ENOENT);
    assert!(err.message.contains("read_file"));
    mock.assert();
}

#[test]
fn test_parse_file_content_response() {
    // Raw file body should be returned as bytes.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/files/data.bin")
        .with_status(200)
        .with_body(vec![0_u8, 1, 2, 250, 255])
        .create();

    let data = run_with_client(server.url(), |client| {
        client.read_file("/data.bin").expect("read should succeed")
    });

    assert_eq!(data, vec![0_u8, 1, 2, 250, 255]);
    mock.assert();
}

#[test]
fn test_parse_empty_response() {
    // Empty 200 body must parse to an empty payload for read_file.
    let mut server = Server::new();
    let mock = server.mock("GET", "/files/empty.txt").with_status(200).with_body("").create();

    let data = run_with_client(server.url(), |client| {
        client
            .read_file("/empty.txt")
            .expect("empty body should be valid")
    });

    assert!(data.is_empty());
    mock.assert();
}

#[test]
fn test_parse_malformed_response() {
    // Malformed JSON body must produce an I/O parsing error.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/list/broken")
        .with_status(200)
        .with_body("{invalid-json")
        .create();

    let err = run_with_client(server.url(), |client| {
        client
            .list_directory("/broken")
            .expect_err("malformed JSON must fail")
    });

    assert_eq!(err.errno, libc::EIO);
    assert!(err.message.contains("Failed to parse response"));
    mock.assert();
}

#[test]
fn test_list_empty_directory() {
    // Empty directory listing must be parsed as an empty vector.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/list/empty")
        .with_status(200)
        .with_body(r#"{"entries":[]}"#)
        .create();

    let entries = run_with_client(server.url(), |client| {
        client
            .list_directory("/empty")
            .expect("empty directory listing should parse")
    });

    assert!(entries.is_empty());
    mock.assert();
}
