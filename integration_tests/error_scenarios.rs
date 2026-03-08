#[path = "../clientFS/src/error.rs"]
mod error;
#[path = "../clientFS/src/api_client.rs"]
mod api_client;

mod common;

use anyhow::Result;
use common::spawn_server;

// Verifica che la lettura di un file inesistente ritorni ENOENT lato client.
#[test]
fn test_read_nonexistent_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let err = client
        .run(|api| api.read_file("/does/not/exist.txt"))
        .expect_err("read_file should fail for missing file");

    assert_eq!(err.errno, libc::ENOENT);
    Ok(())
}

// Verifica che la cancellazione di un file inesistente ritorni ENOENT lato client.
#[test]
fn test_delete_nonexistent_file() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    let err = client
        .run(|api| api.delete("/missing/delete.me"))
        .expect_err("delete should fail for missing file");

    assert_eq!(err.errno, libc::ENOENT);
    Ok(())
}
