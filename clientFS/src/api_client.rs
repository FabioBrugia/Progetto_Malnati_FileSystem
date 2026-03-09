use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::ApiError;

/// Voce di un file/directory restituita dal server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: f64,
    pub ctime: f64,
    pub mode: u32,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    entries: Vec<FileEntry>,
}

#[derive(Serialize)]
struct SetAttrsRequest {
    mode: Option<u32>,
}

/// Client HTTP asincrono per comunicare con il server di storage.
///
/// Utilizza reqwest async internamente, ma espone metodi sincroni
/// tramite block_on() per compatibilità con il trait Filesystem di FUSE.
pub struct ApiClient {
    base_url: String,
    client: Client,
    token: String,
    runtime: tokio::runtime::Handle,
}

impl ApiClient {
    fn validate_path(path: &str, operation: &str) -> Result<(), ApiError> {
        if path
            .split('/')
            .any(|segment| segment == "..")
        {
            return Err(ApiError {
                errno: libc::EINVAL,
                message: format!("{}: Invalid path traversal attempt", operation),
            });
        }

        Ok(())
    }

    /// Crea un client API.
    ///
    /// Argomenti
    ///
    /// base_url - URL base del server
    /// token - Token JWT per l'autenticazione
    /// runtime - Handle al runtime Tokio per eseguire future async
    pub fn new(base_url: String, token: String, runtime: tokio::runtime::Handle) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { base_url, client, token, runtime })
    }

    /// Aggiunge l'header Authorization alla request.
    fn authorized(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header("Authorization", format!("Bearer {}", self.token))
    }

    /// Esegue un future async bloccando il thread corrente.
    /// Usa block_in_place per permettere l'uso anche dall'interno di un runtime Tokio.
    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
        } else {
            self.runtime.block_on(future)
        }
    }

    // Operazioni sul filesystem

    /// Elenca il contenuto di una directory.
    pub fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>, ApiError> {
        Self::validate_path(path, "list_directory")?;
        let url = format!("{}/list/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Listing directory: {}", url);

        let response = self.block_on(
            self.authorized(self.client.get(&url)).send()
        ).map_err(|e| ApiError::from_network_error("list_directory", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("list_directory failed: HTTP {}", status);
            return Err(ApiError::from_status(status, "list_directory"));
        }

        let list_response: ListResponse = self.block_on(response.json())
            .map_err(|e| ApiError::io_error("list_directory", &format!("Failed to parse response: {}", e)))?;

        Ok(list_response.entries)
    }

    /// Legge l'intero contenuto di un file.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        Self::validate_path(path, "read_file")?;
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Reading file: {}", url);

        let response = self.block_on(
            self.authorized(self.client.get(&url)).send()
        ).map_err(|e| ApiError::from_network_error("read_file", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("read_file failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "read_file"));
        }

        let bytes = self.block_on(response.bytes())
            .map_err(|e| ApiError::io_error("read_file", &format!("Failed to read response: {}", e)))?;

        Ok(bytes.to_vec())
    }

    /// Legge un chunk di un file usando HTTP Range requests.
    pub fn read_file_chunk(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, ApiError> {
        Self::validate_path(path, "read_file_chunk")?;
        if size == 0 {
            log::debug!("read_file_chunk called with size=0 for {}, returning empty", path);
            return Ok(Vec::new());
        }

        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        let end = offset
            .checked_add(size as u64 - 1)
            .ok_or_else(|| ApiError::io_error("read_file_chunk", "Invalid range overflow"))?;

        log::debug!("Reading file chunk: {} (offset={}, size={})", url, offset, size);

        let response = self.block_on(
            self.authorized(
                self.client.get(&url)
                    .header("Range", format!("bytes={}-{}", offset, end))
            ).send()
        ).map_err(|e| ApiError::from_network_error("read_file_chunk", &e))?;

        let status = response.status();
        if !status.is_success() && status.as_u16() != 206 {
            log::warn!("read_file_chunk failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "read_file_chunk"));
        }

        let bytes = self.block_on(response.bytes())
            .map_err(|e| ApiError::io_error("read_file_chunk", &format!("Failed to read response: {}", e)))?;

        let result = if bytes.len() > size as usize {
            bytes[0..size as usize].to_vec()
        } else {
            bytes.to_vec()
        };

        log::debug!("Read {} bytes from offset {}", result.len(), offset);
        Ok(result)
    }

    /// Scrive (o sovrascrive) l'intero contenuto di un file.
    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<(), ApiError> {
        Self::validate_path(path, "write_file")?;
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Writing file: {} ({} bytes)", url, data.len());

        let response = self.block_on(
            self.authorized(self.client.put(&url).body(data.to_vec())).send()
        ).map_err(|e| ApiError::from_network_error("write_file", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("write_file failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "write_file"));
        }

        Ok(())
    }

    /// Scrive un chunk di dati a un offset specifico (usa PATCH con Content-Range).
    ///
    /// Se il server non supporta PATCH (405), fallback a read-modify-write.
    pub fn write_file_chunk(&self, path: &str, offset: u64, data: &[u8]) -> Result<(), ApiError> {
        Self::validate_path(path, "write_file_chunk")?;
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Writing file chunk: {} (offset={}, size={})", url, offset, data.len());

        if data.is_empty() {
            log::debug!("write_file_chunk called with empty payload for {}, nothing to write", path);
            return Ok(());
        }

        let end = offset
            .checked_add(data.len() as u64 - 1)
            .ok_or_else(|| ApiError::io_error("write_file_chunk", "Invalid range overflow"))?;
        let response = self.block_on(
            self.authorized(
                self.client.patch(&url)
                    .header("Content-Range", format!("bytes {}-{}/*", offset, end))
                    .body(data.to_vec())
            ).send()
        ).map_err(|e| ApiError::from_network_error("write_file_chunk", &e))?;

        let status = response.status();
        if status.is_success() {
            log::debug!("Successfully wrote chunk using PATCH");
            return Ok(());
        }

        if status.as_u16() == 405 {
            log::warn!("Server doesn't support PATCH, falling back to read-modify-write");
            return self.write_file_chunk_fallback(path, offset, data);
        }

        log::warn!("write_file_chunk failed for {}: HTTP {}", path, status);
        Err(ApiError::from_status(status, "write_file_chunk"))
    }

    /// NOT USED: Fallback: legge tutto il file, modifica in memoria, riscrive tutto.
    fn write_file_chunk_fallback(&self, path: &str, offset: u64, data: &[u8]) -> Result<(), ApiError> {
        log::warn!("Using inefficient read-modify-write for {}", path);

        let mut file_data = match self.read_file(path) {
            Ok(existing) => existing,
            Err(err) if err.errno == libc::ENOENT => Vec::new(),
            Err(err) => {
                log::warn!("Fallback read failed for {}: {}", path, err);
                return Err(err);
            }
        };

        let end_offset = (offset as usize) + data.len();
        if end_offset > file_data.len() {
            file_data.resize(end_offset, 0);
        }

        file_data[offset as usize..end_offset].copy_from_slice(data);
        self.write_file(path, &file_data)
    }

    /// Crea una nuova directory sul server.
    pub fn create_directory(&self, path: &str) -> Result<(), ApiError> {
        Self::validate_path(path, "create_directory")?;
        let url = format!("{}/mkdir/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Creating directory: {}", url);

        let response = self.block_on(
            self.authorized(self.client.post(&url)).send()
        ).map_err(|e| ApiError::from_network_error("create_directory", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("create_directory failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "create_directory"));
        }

        Ok(())
    }

    /// Elimina un file o una directory sul server.
    pub fn delete(&self, path: &str) -> Result<(), ApiError> {
        Self::validate_path(path, "delete")?;
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Deleting: {}", url);

        let response = self.block_on(
            self.authorized(self.client.delete(&url)).send()
        ).map_err(|e| ApiError::from_network_error("delete", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("delete failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "delete"));
        }

        Ok(())
    }

    /// Rinomina un file o directory sul server.
    pub fn rename(&self, from: &str, to: &str) -> Result<(), ApiError> {
        Self::validate_path(from, "rename")?;
        Self::validate_path(to, "rename")?;
        let url = format!("{}/rename", self.base_url);
        log::debug!("Renaming: {} -> {}", from, to);

        #[derive(Serialize)]
        struct RenameRequest {
            from: String,
            to: String,
        }

        let request_body = RenameRequest {
            from: from.to_string(),
            to: to.to_string(),
        };

        let response = self.block_on(
            self.authorized(self.client.post(&url).json(&request_body)).send()
        ).map_err(|e| ApiError::from_network_error("rename", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("rename failed ({} -> {}): HTTP {}", from, to, status);
            return Err(ApiError::from_status(status, "rename"));
        }

        Ok(())
    }

    /// Imposta gli attributi di un file sul server.
    pub fn set_attrs(&self, path: &str, mode: Option<u32>) -> Result<(), ApiError> {
        Self::validate_path(path, "set_attrs")?;
        let url = format!("{}/attrs/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Setting attrs for {}: mode={:?}", url, mode);

        let request_body = SetAttrsRequest { mode };

        let response = self.block_on(
            self.authorized(self.client.patch(&url).json(&request_body)).send()
        ).map_err(|e| ApiError::from_network_error("set_attrs", &e))?;

        let status = response.status();
        if !status.is_success() {
            log::warn!("set_attrs failed for {}: HTTP {}", path, status);
            return Err(ApiError::from_status(status, "set_attrs"));
        }

        Ok(())
    }

    /// Verifica la connessione al server.
    pub fn health_check(&self) -> Result<(), ApiError> {
        let url = format!("{}/health", self.base_url);

        let response = self.block_on(
            self.authorized(self.client.get(&url)).send()
        ).map_err(|e| ApiError::from_network_error("health_check", &e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::from_status(status, "health_check"));
        }

        Ok(())
    }
}
