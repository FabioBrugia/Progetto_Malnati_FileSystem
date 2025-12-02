use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

pub struct ApiClient {
    base_url: String,
    client: Client,
}

impl ApiClient {
    pub fn new(base_url: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { base_url, client })
    }

    pub fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>> {
        let url = format!("{}/list/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Listing directory: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to send list request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        let list_response: ListResponse = response
            .json()
            .context("Failed to parse list response")?;

        Ok(list_response.entries)
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Reading file: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to send read request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        let bytes = response.bytes().context("Failed to read response")?;
        Ok(bytes.to_vec())
    }

    pub fn read_file_chunk(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>> {
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        let end = offset + size as u64 - 1;

        log::debug!("Reading file chunk: {} (offset={}, size={})", url, offset, size);

        let response = self
            .client
            .get(&url)
            .header("Range", format!("bytes={}-{}", offset, end))
            .send()
            .context("Failed to send range read request")?;

        let status = response.status();
        if !status.is_success() && status.as_u16() != 206 {
            anyhow::bail!("Server returned error: {}", status);
        }

        let bytes = response.bytes().context("Failed to read response")?;

        let result = if bytes.len() > size as usize {
            bytes[0..size as usize].to_vec()
        } else {
            bytes.to_vec()
        };

        log::debug!("Read {} bytes from offset {}", result.len(), offset);
        Ok(result)
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<()> {
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Writing file: {} ({} bytes)", url, data.len());

        let response = self
            .client
            .put(&url)
            .body(data.to_vec())
            .send()
            .context("Failed to send write request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        Ok(())
    }

    pub fn write_file_chunk(&self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Writing file chunk: {} (offset={}, size={})", url, offset, data.len());

        let end = offset + data.len() as u64 - 1;
        let response = self
            .client
            .patch(&url)
            .header("Content-Range", format!("bytes {}-{}/*", offset, end))
            .body(data.to_vec())
            .send()
            .context("Failed to send PATCH request")?;

        if response.status().is_success() {
            log::debug!("Successfully wrote chunk using PATCH");
            return Ok(());
        }

        if response.status().as_u16() == 405 {
            log::warn!("Server doesn't support PATCH, falling back to read-modify-write");
            return self.write_file_chunk_fallback(path, offset, data);
        }

        anyhow::bail!("Server returned error: {}", response.status());
    }

    fn write_file_chunk_fallback(&self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        log::warn!("Using inefficient read-modify-write for {}", path);

        let mut file_data = self.read_file(path).unwrap_or_else(|_| Vec::new());

        let end_offset = (offset as usize) + data.len();
        if end_offset > file_data.len() {
            file_data.resize(end_offset, 0);
        }

        file_data[offset as usize..end_offset].copy_from_slice(data);

        self.write_file(path, &file_data)
    }

    pub fn create_directory(&self, path: &str) -> Result<()> {
        let url = format!("{}/mkdir/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Creating directory: {}", url);

        let response = self
            .client
            .post(&url)
            .send()
            .context("Failed to send mkdir request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        Ok(())
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}/files/{}", self.base_url, path.trim_start_matches('/'));
        log::debug!("Deleting: {}", url);

        let response = self
            .client
            .delete(&url)
            .send()
            .context("Failed to send delete request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
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

        let response = self
            .client
            .post(&url)
            .json(&request_body)
            .send()
            .context("Failed to send rename request")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        Ok(())
    }

    pub fn health_check(&self) -> Result<()> {
        let url = format!("{}/health", self.base_url);
        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            anyhow::bail!("Health check failed");
        }

        Ok(())
    }
}
