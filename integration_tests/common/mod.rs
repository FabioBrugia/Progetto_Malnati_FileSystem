use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::env;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use crate::api_client::ApiClient;

const TEST_PASSWORD: &str = "1234";

#[derive(Debug, Deserialize)]
struct AuthResponse {
    token: String,
}

pub struct ClientCtx {
    runtime: tokio::runtime::Runtime,
    pub api: ApiClient,
}

impl ClientCtx {
    fn new(base_url: String, token: String) -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new().context("failed to build tokio runtime")?;
        let api = ApiClient::new(base_url, token, runtime.handle().clone())
            .context("failed to create ApiClient")?;

        Ok(Self {
            runtime,
            api,
        })
    }

    pub fn run<T>(&self, operation: impl FnOnce(&ApiClient) -> T) -> T {
        self.runtime.block_on(async { operation(&self.api) })
    }
}

pub struct TestServer {
    child: Child,
    _storage_dir: TempDir,
    pub base_url: String,
    token: String,
}

impl TestServer {
    pub fn client(&self) -> Result<ClientCtx> {
        ClientCtx::new(self.base_url.clone(), self.token.clone())
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn spawn_server() -> Result<TestServer> {
    let storage_dir = TempDir::new().context("failed to create temp storage directory")?;
    let port = pick_free_port().context("failed to pick a free TCP port")?;
    let bind_addr = format!("127.0.0.1:{port}");
    let base_url = format!("http://{bind_addr}");

    let server_bin = find_server_binary();

    let child = Command::new(&server_bin)
        .env("SERVER_BASE_DIR", storage_dir.path())
        .env("SERVER_BIND_ADDR", &bind_addr)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start server binary at {server_bin}"))?;

    let token = wait_for_server_and_auth(&base_url, child.id())?;

    Ok(TestServer {
        child,
        _storage_dir: storage_dir,
        base_url,
        token,
    })
}

fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind ephemeral port")?;
    let port = listener
        .local_addr()
        .context("failed to inspect local address")?
        .port();
    Ok(port)
}

fn find_server_binary() -> String {
    if let Ok(path) = env::var("CARGO_BIN_EXE_server") {
        return path;
    }

    let mut path = env::current_exe().unwrap_or_else(|_| "target/debug/deps/test".into());
    let _ = path.pop(); // test binary filename
    if path.ends_with("deps") {
        let _ = path.pop(); // target/debug
    }
    path.push("server");
    path.to_string_lossy().to_string()
}

fn wait_for_server_and_auth(base_url: &str, child_pid: u32) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .context("failed to create bootstrap HTTP client")?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(response) = client
            .post(format!("{base_url}/auth"))
            .json(&serde_json::json!({ "password": TEST_PASSWORD }))
            .send()
        {
            if response.status().is_success() {
                let auth: AuthResponse = response
                    .json()
                    .context("server returned invalid auth payload")?;
                return Ok(auth.token);
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!("server (pid {child_pid}) did not become ready before timeout"))
}
