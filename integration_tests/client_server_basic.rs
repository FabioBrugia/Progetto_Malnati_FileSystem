#[path = "../clientFS/src/error.rs"]
mod error;
#[path = "../clientFS/src/api_client.rs"]
mod api_client;

mod common;

use anyhow::Result;
use common::spawn_server;

// Verifica che il server si avvii correttamente e accetti connessioni client autenticate.
#[test]
fn test_server_startup() -> Result<()> {
    let server = spawn_server()?;
    let client = server.client()?;

    client.run(|api| api.health_check())?;

    Ok(())
}
