#[path = "../clientFS/src/error.rs"]
mod error;
#[path = "../clientFS/src/api_client.rs"]
mod api_client;

mod common;

use anyhow::Result;
use common::spawn_server;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

fn send_raw_authenticated_request(
    server: &common::TestServer,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<u16> {
    let host_port = server.base_url.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;

    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        server.token(),
        body.len()
    );

    stream.write_all(request.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.shutdown(Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| anyhow::anyhow!("missing HTTP status line"))?
        .parse::<u16>()?;

    Ok(status)
}

// Verifica il workflow completo create -> write -> read su un file.
#[test]
fn test_create_and_read_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/docs/readme.txt";
    let content = b"hello from integration test";

    client.run(|api| api.write_file(path, content))?;
    let read_back = client.run(|api| api.read_file(path))?;

    assert_eq!(read_back, content);

    let entries = client.run(|api| api.list_directory("/docs"))?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "readme.txt");
    assert!(!entries[0].is_dir);
    Ok(())
}

// Verifica che la creazione di una directory remota vada a buon fine.
#[test]
fn test_create_directory() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    client.run(|api| api.create_directory("/projects"))?;

    let root_entries = client.run(|api| api.list_directory("/"))?;
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0].name, "projects");
    assert!(root_entries[0].is_dir);

    Ok(())
}

// Verifica che il listing includa le directory create dal client.
#[test]
fn test_list_directory() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    client.run(|api| api.create_directory("/workspace"))?;
    client.run(|api| api.write_file("/workspace/a.txt", b"a"))?;
    client.run(|api| api.write_file("/workspace/b.txt", b"b"))?;

    let entries = client.run(|api| api.list_directory("/workspace"))?;
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| !entry.is_dir));
    assert!(entries.iter().any(|entry| entry.name == "a.txt" && entry.size == 1));
    assert!(entries.iter().any(|entry| entry.name == "b.txt" && entry.size == 1));

    Ok(())
}

// Verifica che il listing di una directory vuota restituisca zero elementi.
#[test]
fn test_list_empty_directory() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    client.run(|api| api.create_directory("/empty"))?;
    let entries = client.run(|api| api.list_directory("/empty"))?;

    assert!(entries.is_empty());
    Ok(())
}

// Verifica delete file e che la risorsa non sia piu leggibile dopo la rimozione.
#[test]
fn test_delete_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/tmp/data.bin";
    client.run(|api| api.write_file(path, b"to-remove"))?;
    client.run(|api| api.delete(path))?;

    let err = client
        .run(|api| api.read_file(path))
        .expect_err("read_file should fail after delete");
    assert_eq!(err.errno, libc::ENOENT);

    let entries = client.run(|api| api.list_directory("/tmp"))?;
    assert!(entries.is_empty());

    Ok(())
}

// Verifica che una seconda write sullo stesso path sovrascriva i dati precedenti.
#[test]
fn test_write_overwrites_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/overwrite.txt";
    client.run(|api| api.write_file(path, b"content-A"))?;
    client.run(|api| api.write_file(path, b"content-B"))?;

    let read_back = client.run(|api| api.read_file(path))?;
    assert_eq!(read_back, b"content-B");

    let entries = client.run(|api| api.list_directory("/"))?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "overwrite.txt");
    assert_eq!(entries[0].size as usize, b"content-B".len());

    Ok(())
}

// Verifica la gestione end-to-end di file grandi (diversi MB).
#[test]
fn test_large_file_write_read() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/large/payload.bin";
    let size = 3 * 1024 * 1024;
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();

    client.run(|api| api.write_file(path, &data))?;
    let read_back = client.run(|api| api.read_file(path))?;

    assert_eq!(read_back.len(), data.len());
    assert_eq!(read_back, data);

    Ok(())
}

// Verifica esplicita della PUT /files con controllo dello stato finale del filesystem.
#[test]
fn test_write_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/explicit_write/data.txt";
    let payload = b"payload-write";
    client.run(|api| api.write_file(path, payload))?;

    let entries = client.run(|api| api.list_directory("/explicit_write"))?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "data.txt");
    assert!(!entries[0].is_dir);
    assert_eq!(entries[0].size as usize, payload.len());

    Ok(())
}

// Verifica esplicita della GET /files con confronto esatto del contenuto.
#[test]
fn test_read_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/explicit_read/data.txt";
    let payload = b"payload-read";
    client.run(|api| api.write_file(path, payload))?;

    let read_back = client.run(|api| api.read_file(path))?;
    assert_eq!(read_back, payload);

    Ok(())
}

// Verifica rifiuto di path traversal in lettura.
#[test]
fn test_path_traversal_read_rejected() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let client_err = client
        .run(|api| api.read_file("../../etc/passwd"))
        .expect_err("client must reject traversal path before request");
    assert_eq!(client_err.errno, libc::EINVAL);

    let status = send_raw_authenticated_request(&server, "GET", "/files/../../etc/passwd", b"")?;

    assert_eq!(status, 400);

    Ok(())
}

// Verifica rifiuto di path traversal in scrittura.
#[test]
fn test_path_traversal_write_rejected() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let client_err = client
        .run(|api| api.write_file("../server_storage/file", b"malicious"))
        .expect_err("client must reject traversal path before request");
    assert_eq!(client_err.errno, libc::EINVAL);

    let status = send_raw_authenticated_request(
        &server,
        "PUT",
        "/files/../server_storage/file",
        b"malicious",
    )?;

    assert_eq!(status, 400);

    Ok(())
}

// Verifica rifiuto di path traversal nella creazione directory.
#[test]
fn test_path_traversal_directory_rejected() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let client_err = client
        .run(|api| api.create_directory("../../tmp/file"))
        .expect_err("client must reject traversal path before request");
    assert_eq!(client_err.errno, libc::EINVAL);

    let status =
        send_raw_authenticated_request(&server, "POST", "/mkdir/../../tmp/file", b"")?;

    assert_eq!(status, 400);

    Ok(())
}

// Verifica file di grandi dimensioni: 100 MB con integrita completa write/read.
#[test]
fn test_large_file_write_read_100mb() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let path = "/large_100mb/payload.bin";
    let size = 100 * 1024 * 1024;
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();

    client.run(|api| api.write_file(path, &data))?;
    let read_back = client.run(|api| api.read_file(path))?;

    assert_eq!(read_back.len(), data.len());
    assert_eq!(read_back, data);

    Ok(())
}

// Verifica requisiti di latenza su operazioni comuni in condizioni normali.
#[test]
fn test_operation_latency_under_500ms() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    client.run(|api| api.write_file("/latency/small.txt", b"small payload"))?;

    let list_started = Instant::now();
    let entries = client.run(|api| api.list_directory("/latency"))?;
    let list_elapsed = list_started.elapsed();
    assert_eq!(entries.len(), 1);
    assert!(list_elapsed < Duration::from_millis(500));

    let read_started = Instant::now();
    let read_back = client.run(|api| api.read_file("/latency/small.txt"))?;
    let read_elapsed = read_started.elapsed();
    assert_eq!(read_back, b"small payload");
    assert!(read_elapsed < Duration::from_millis(500));

    Ok(())
}
