#[path = "../src/error.rs"]
mod error;
#[path = "../src/api_client.rs"]
mod api_client;
mod common;

use api_client::FileEntry;
use common::run_with_client;
use mockito::{Matcher, Server};

#[test]
fn test_request_serialization() {
    // Ensures PATCH wire format contains Content-Range and exact chunk payload.
    let mut server = Server::new();
    let mock = server
        .mock("PATCH", "/files/chunks/data.bin")
        .match_header("authorization", "Bearer test-token")
        .match_header("content-range", "bytes 5-7/*")
        .match_body(Matcher::Exact("abc".to_string()))
        .with_status(204)
        .create();

    let result = run_with_client(server.url(), |client| {
        client.write_file_chunk("/chunks/data.bin", 5, b"abc")
    });

    assert!(result.is_ok());
    mock.assert();
}

#[test]
fn test_request_deserialization() {
    // Ensures request path normalization keeps protocol contract stable.
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/files/dir/file.txt")
        .match_header("range", "bytes=8-10")
        .with_status(206)
        .with_body("xyz")
        .create();

    let data = run_with_client(server.url(), |client| {
        client
            .read_file_chunk("/dir/file.txt", 8, 3)
            .expect("chunk read must succeed")
    });

    assert_eq!(data, b"xyz");
    mock.assert();
}

#[test]
fn test_response_serialization() {
    // Verifies FileEntry serde serialization shape used by the client protocol.
    let entry = FileEntry {
        name: "hello.txt".to_string(),
        is_dir: false,
        size: 42,
        mtime: 1000.5,
        ctime: 900.0,
        mode: 0o644,
    };

    let value = serde_json::to_value(&entry).expect("serialization must succeed");
    assert_eq!(value["name"], "hello.txt");
    assert_eq!(value["is_dir"], false);
    assert_eq!(value["size"], 42);
    assert_eq!(value["mode"], 420);
}

#[test]
fn test_response_deserialization() {
    // Verifies FileEntry serde deserialization from wire-compatible JSON.
    let json = r#"{"name":"dir","is_dir":true,"size":0,"mtime":10.0,"ctime":5.0,"mode":493}"#;
    let entry: FileEntry = serde_json::from_str(json).expect("deserialization must succeed");

    assert_eq!(entry.name, "dir");
    assert!(entry.is_dir);
    assert_eq!(entry.size, 0);
    assert_eq!(entry.mode, 493);
}
