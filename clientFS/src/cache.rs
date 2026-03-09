use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use crate::api_client::{ApiClient, FileEntry};
use crate::error::ApiError;

/// Dimensione dei chunk per la lettura a blocchi (128KB)
pub const CHUNK_SIZE: u32 = 128 * 1024;
/// Dimensione massima della cache dati in memoria (10MB)
const MAX_CACHE_SIZE: usize = 10 * 1024 * 1024;

// Configurazione TTL delle varie cache
/// TTL per i metadati degli inode
pub const METADATA_CACHE_TTL: Duration = Duration::from_secs(5);
/// TTL per il listing delle directory
pub const DIRECTORY_CACHE_TTL: Duration = Duration::from_secs(3);
/// TTL per i dati dei file (chunk)
pub const DATA_CACHE_TTL: Duration = Duration::from_secs(10);

//Wrapper  con TTL

/// Entry di cache con scadenza temporale (TTL).
#[derive(Debug, Clone)]
pub struct CachedEntry<T> {
    pub data: T,
    created_at: Instant,
    ttl: Duration,
}

impl<T> CachedEntry<T> {
    pub fn new(data: T, ttl: Duration) -> Self {
        Self {
            data,
            created_at: Instant::now(),
            ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

//Cache chunk dati file

/// Chunk di dati di un file in cache, con LRU.
#[derive(Debug, Clone)]
struct CachedChunk {
    data: Vec<u8>,
    #[allow(dead_code)]
    offset: u64,
    last_access: SystemTime,
    created_at: Instant,
}

impl CachedChunk {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > DATA_CACHE_TTL
    }
}

/// Cache dei chunk per un singolo file.
#[derive(Debug, Clone)]
struct FileCache {
    chunks: HashMap<u64, CachedChunk>,
    total_size: usize,
}

// Cache listing directory

/// Cache per il listing di una directory.
#[derive(Debug, Clone)]
pub struct DirectoryCache {
    pub entries: CachedEntry<Vec<FileEntry>>,
}

// Cache Manager

/// Gestore di tutte le cache del filesystem.
///
/// Gestisce 2 livelli di cache:
/// - Directory cache: listing directory con TTL
/// - file data cache: chunk di dati file con LRU + TTL
pub struct CacheManager {
    /// Cache dei dati file (chunk)
    file_cache: HashMap<String, FileCache>,
    /// Cache dei listing directory
    directory_cache: HashMap<String, DirectoryCache>,
}

impl CacheManager {
    pub fn new() -> Self {
        Self {
            file_cache: HashMap::new(),
            directory_cache: HashMap::new(),
        }
    }

    // Directory cache

    /// Restituisce il listing directory solo se presente in cache e valido.
    ///
    /// Non effettua chiamate remote.
    pub fn get_cached_directory(&mut self, path: &str) -> Option<Vec<FileEntry>> {
        if let Some(dir_cache) = self.directory_cache.get(path) {
            if !dir_cache.entries.is_expired() {
                log::debug!("Directory cache HIT for {}", path);
                return Some(dir_cache.entries.data.clone());
            }
            log::debug!("Directory cache EXPIRED for {}", path);
        }

        self.directory_cache.remove(path);
        None
    }

    /// Salva il listing di una directory in cache.
    pub fn store_directory_listing(&mut self, path: &str, entries: Vec<FileEntry>) {
        self.directory_cache.insert(
            path.to_string(),
            DirectoryCache {
                entries: CachedEntry::new(entries, DIRECTORY_CACHE_TTL),
            },
        );
    }

    /// Ottiene il listing di una directory dalla cache o dal server.
    pub fn list_directory_cached(
        &mut self,
        path: &str,
        api_client: &ApiClient,
    ) -> Result<Vec<FileEntry>, ApiError> {
        if let Some(entries) = self.get_cached_directory(path) {
            return Ok(entries);
        }

        // Cache miss o expired — fetch dal server
        log::debug!("Directory cache MISS for {}", path);
        let entries = api_client.list_directory(path)?;

        self.store_directory_listing(path, entries.clone());

        Ok(entries)
    }

    /// Invalida la cache di una directory e della sua directory parent.
    pub fn invalidate_directory_cache(&mut self, path: &str) {
        if self.directory_cache.remove(path).is_some() {
            log::debug!("Invalidated directory cache for {}", path);
        }

        // Invalida anche la directory parent
        if let Some(parent_pos) = path.rfind('/') {
            let parent = if parent_pos == 0 { "/" } else { &path[..parent_pos] };
            if self.directory_cache.remove(parent).is_some() {
                log::debug!("Invalidated parent directory cache for {}", parent);
            }
        }
    }

    // File data cache

    /// Legge dati solo dalla cache locale (nessun I/O remoto).
    pub fn read_from_cache(&mut self, path: &str, offset: u64, size: u32) -> Option<Vec<u8>> {
        let chunk_start = (offset / CHUNK_SIZE as u64) * CHUNK_SIZE as u64;

        if let Some(file_cache) = self.file_cache.get_mut(path) {
            if let Some(cached_chunk) = file_cache.chunks.get_mut(&chunk_start) {
                if !cached_chunk.is_expired() {
                    cached_chunk.last_access = SystemTime::now();
                    let chunk_offset = (offset - chunk_start) as usize;
                    let chunk_end = (chunk_offset + size as usize).min(cached_chunk.data.len());

                    log::debug!("Cache HIT for {} at offset {} (TTL valid)", path, offset);
                    return Some(cached_chunk.data[chunk_offset..chunk_end].to_vec());
                }

                let removed = file_cache.chunks.remove(&chunk_start);
                if let Some(chunk) = removed {
                    file_cache.total_size -= chunk.data.len();
                    log::debug!("Cache EXPIRED for {} at offset {}", path, offset);
                }
            }
        }

        None
    }

    /// Salva un chunk letto dal server nella cache locale.
    pub fn store_file_chunk(&mut self, path: &str, offset: u64, data: Vec<u8>) {
        self.store_chunk(path, offset, data);
    }

    /// Legge dati di un file dalla cache o dal server, usando range requests.
    pub fn read_with_cache(
        &mut self,
        path: &str,
        offset: u64,
        size: u32,
        api_client: &ApiClient,
    ) -> Result<Vec<u8>, ApiError> {
        let chunk_start = (offset / CHUNK_SIZE as u64) * CHUNK_SIZE as u64;

        if let Some(data) = self.read_from_cache(path, offset, size) {
            return Ok(data);
        }

        // Cache miss — fetch dal server
        log::debug!("Cache MISS for {} at offset {}", path, offset);
        let chunk_data = api_client.read_file_chunk(path, chunk_start, CHUNK_SIZE)?;

        // Salva in cache
        self.store_file_chunk(path, chunk_start, chunk_data.clone());

        // Estrai la porzione richiesta
        let chunk_offset = (offset - chunk_start) as usize;
        let chunk_end = (chunk_offset + size as usize).min(chunk_data.len());
        Ok(chunk_data[chunk_offset..chunk_end].to_vec())
    }

    /// Salva un chunk nella cache, evitando vecchi entry se necessario (LRU).
    fn store_chunk(&mut self, path: &str, offset: u64, data: Vec<u8>) {
        let file_cache = self.file_cache.entry(path.to_string()).or_insert_with(|| FileCache {
            chunks: HashMap::new(),
            total_size: 0,
        });

        // Rimuovi chunk scaduti
        let expired_offsets: Vec<u64> = file_cache
            .chunks
            .iter()
            .filter(|(_, chunk)| chunk.is_expired())
            .map(|(&offset, _)| offset)
            .collect();

        for expired_offset in expired_offsets {
            if let Some(removed) = file_cache.chunks.remove(&expired_offset) {
                file_cache.total_size -= removed.data.len();
                log::debug!("Evicted expired chunk at offset {} from cache", expired_offset);
            }
        }

        // evitare LRU se la cache è troppo grande
        while file_cache.total_size + data.len() > MAX_CACHE_SIZE && !file_cache.chunks.is_empty() {
            if let Some((&oldest_offset, _)) = file_cache
                .chunks
                .iter()
                .min_by_key(|(_, chunk)| chunk.last_access)
            {
                if let Some(removed) = file_cache.chunks.remove(&oldest_offset) {
                    file_cache.total_size -= removed.data.len();
                    log::debug!("Evicted LRU chunk at offset {} from cache", oldest_offset);
                }
            } else {
                break;
            }
        }

        // Aggiungi il nuovo chunk
        let chunk_size = data.len();
        if let Some(previous) = file_cache.chunks.remove(&offset) {
            file_cache.total_size = file_cache.total_size.saturating_sub(previous.data.len());
        }

        file_cache.chunks.insert(offset, CachedChunk {
            data,
            offset,
            last_access: SystemTime::now(),
            created_at: Instant::now(),
        });
        file_cache.total_size += chunk_size;
        log::debug!(
            "Cached chunk at offset {} for {} ({} bytes, TTL {}s)",
            offset, path, chunk_size, DATA_CACHE_TTL.as_secs()
        );
    }

    /// Invalida la cache dati di un file.
    pub fn invalidate_file_cache(&mut self, path: &str) {
        if let Some(removed) = self.file_cache.remove(path) {
            log::debug!("Invalidated cache for {} ({} chunks)", path, removed.chunks.len());
        }
    }

    // Invalidazione globale


    /// Write-through chiamato dopo operazioni di scrittura per garantire consistenza.
    pub fn invalidate_all_for_path(&mut self, path: &str) {
        self.invalidate_file_cache(path);
        self.invalidate_directory_cache(path);
        log::debug!("Invalidated all caches for {}", path);
    }

    // Pulizia periodica

    /// Pulisce tutte le entry scadute dalle cache.
    ///
    /// Deve essere chiamato periodFileEntryicamente (es. ad ogni getattr sulla root).
    pub fn cleanup_expired(&mut self) {
        // Pulisci directory cache
        self.directory_cache.retain(|path, dir_cache| {
            let keep = !dir_cache.entries.is_expired();
            if !keep {
                log::debug!("Cleaned up expired directory cache for {}", path);
            }
            keep
        });

        // Pulisci file chunk cache
        for (path, file_cache) in self.file_cache.iter_mut() {
            let old_count = file_cache.chunks.len();
            file_cache.chunks.retain(|offset, chunk| {
                let keep = !chunk.is_expired();
                if !keep {
                    file_cache.total_size -= chunk.data.len();
                    log::debug!("Cleaned up expired chunk at offset {} for {}", offset, path);
                }
                keep
            });
            if file_cache.chunks.len() < old_count {
                log::debug!(
                    "Cleaned up {} expired chunks for {}",
                    old_count - file_cache.chunks.len(),
                    path
                );
            }
        }

        // Rimuovi file senza chunks rimasti
        self.file_cache.retain(|_, file_cache| !file_cache.chunks.is_empty());
    }

    /// Svuota completamente tutte le cache.
    ///
    /// Chiamato durante il graceful shutdown del filesystem.
    pub fn clear(&mut self) {
        self.file_cache.clear();
        self.directory_cache.clear();
    }
}

