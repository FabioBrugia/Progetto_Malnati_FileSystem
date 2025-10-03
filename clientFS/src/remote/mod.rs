use std::error::Error;
use serde::{Deserialize, Serialize};
use reqwest::Client as HttpClient;
use std::time::SystemTime;

pub struct Client {
    base_url: String,
    http_client: HttpClient,
}

impl Client {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            http_client: HttpClient::new(),
        }
    }

    pub async fn get_file_info(&self, path: &str) -> Result<FileInfo, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/files{}", self.base_url, path);
        let response = self.http_client.head(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("File not found: {}", path).into());
        }

        let size = response.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let modified = response.headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| httpdate::parse_http_date(s).ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        Ok(FileInfo {
            size,
            is_dir: false,
            modified,
        })
    }

    pub async fn read_file(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/files{}", self.base_url, path);
        let range = format!("bytes={}-{}", offset, offset + size as u64 - 1);

        let response = self.http_client
            .get(&url)
            .header("Range", range)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Failed to read file: {}", path).into());
        }

        Ok(response.bytes().await?.to_vec())
    }

    pub async fn write_file(&self, path: &str, _offset: u64, data: &[u8]) -> Result<u32, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/files{}", self.base_url, path);

        let response = self.http_client
            .put(&url)
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("Failed to write file: {}", path).into());
        }

        Ok(data.len() as u32)
    }

    pub async fn list_directory(&self, path: &str) -> Result<Vec<DirEntry>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/list{}", self.base_url, path);

        let response = self.http_client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to list directory: {}", path).into());
        }

        let entries: Vec<ApiDirEntry> = response.json().await?;
        Ok(entries.into_iter().map(|e| DirEntry {
            name: e.name,
            is_dir: e.is_dir,
        }).collect())
    }

    pub async fn create_directory(&self, path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/mkdir{}", self.base_url, path);

        let response = self.http_client.post(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to create directory: {}", path).into());
        }

        Ok(())
    }

    pub async fn delete_file(&self, path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/files{}", self.base_url, path);

        let response = self.http_client.delete(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to delete file: {}", path).into());
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub size: u64,
    pub is_dir: bool,
    pub modified: SystemTime,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Deserialize)]
struct ApiDirEntry {
    name: String,
    #[serde(rename = "isDirectory")]
    is_dir: bool,
}
