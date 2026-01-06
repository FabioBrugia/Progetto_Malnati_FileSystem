use fuser::{
    Filesystem,
    FileAttr,
    FileType,
    ReplyAttr,
    ReplyData,
    ReplyDirectory,
    ReplyEntry,
    ReplyEmpty,
    ReplyWrite,
    ReplyCreate,
    Request,
    MountOption,
};
use libc::ENOENT;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::api_client::{ApiClient, FileEntry};

const TTL: Duration = Duration::from_secs(1);
const CHUNK_SIZE: u32 = 128 * 1024; // 128KB chunks for streaming
const MAX_CACHE_SIZE: usize = 10 * 1024 * 1024; // 10MB cache

#[derive(Debug, Clone)]
struct CachedChunk {
    data: Vec<u8>,
    offset: u64,
    last_access: SystemTime,
}

#[derive(Debug, Clone)]
struct FileCache {
    chunks: HashMap<u64, CachedChunk>, // key is chunk start offset
    total_size: usize,
}

#[derive(Debug, Clone)]
struct INode {
    ino: u64,
    path: String,
    attr: FileAttr,
}

pub struct RemoteFS {
    api_client: Arc<ApiClient>,
    inodes: Arc<Mutex<HashMap<u64, INode>>>,
    path_to_ino: Arc<Mutex<HashMap<String, u64>>>,
    next_ino: Arc<Mutex<u64>>,
    file_handles: Arc<Mutex<HashMap<u64, String>>>,
    #[allow(dead_code)]
    next_fh: Arc<Mutex<u64>>,
    // Cache for file data chunks
    file_cache: Arc<Mutex<HashMap<String, FileCache>>>,
}

impl RemoteFS {
    pub fn new(api_client: ApiClient) -> Self {
        let mut inodes = HashMap::new();
        let mut path_to_ino = HashMap::new();

        // Create root inode
        let root_attr = FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        let root_inode = INode {
            ino: 1,
            path: "/".to_string(),
            attr: root_attr,
        };

        inodes.insert(1, root_inode);
        path_to_ino.insert("/".to_string(), 1);

        Self {
            api_client: Arc::new(api_client),
            inodes: Arc::new(Mutex::new(inodes)),
            path_to_ino: Arc::new(Mutex::new(path_to_ino)),
            next_ino: Arc::new(Mutex::new(2)),
            file_handles: Arc::new(Mutex::new(HashMap::new())),
            next_fh: Arc::new(Mutex::new(1)),
            file_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_or_create_inode(&self, path: &str, entry: &FileEntry) -> u64 {
        let mut path_to_ino = self.path_to_ino.lock().unwrap();
        let mut inodes = self.inodes.lock().unwrap();
        let mut next_ino = self.next_ino.lock().unwrap();

        if let Some(&ino) = path_to_ino.get(path) {
            return ino;
        }

        let ino = *next_ino;
        *next_ino += 1;

        let attr = FileAttr {
            ino,
            size: entry.size,
            blocks: (entry.size + 511) / 512,
            atime: UNIX_EPOCH + Duration::from_secs_f64(entry.mtime),
            mtime: UNIX_EPOCH + Duration::from_secs_f64(entry.mtime),
            ctime: UNIX_EPOCH + Duration::from_secs_f64(entry.ctime),
            crtime: UNIX_EPOCH + Duration::from_secs_f64(entry.ctime),
            kind: if entry.is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: (entry.mode & 0o777) as u16,
            nlink: if entry.is_dir { 2 } else { 1 },
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        let inode = INode {
            ino,
            path: path.to_string(),
            attr,
        };

        inodes.insert(ino, inode);
        path_to_ino.insert(path.to_string(), ino);

        ino
    }

    fn get_inode(&self, ino: u64) -> Option<INode> {
        let inodes = self.inodes.lock().unwrap();
        inodes.get(&ino).cloned()
    }

    fn path_from_parent_and_name(&self, parent: u64, name: &OsStr) -> Option<String> {
        let inodes = self.inodes.lock().unwrap();
        let parent_inode = inodes.get(&parent)?;
        let name_str = name.to_str()?;

        let parent_path = &parent_inode.path;
        let path = if parent_path == "/" {
            format!("/{}", name_str)
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        Some(path)
    }

    /// Get data from cache or fetch from server using range requests
    fn read_with_cache(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, crate::api_client::ApiError> {
        let chunk_start = (offset / CHUNK_SIZE as u64) * CHUNK_SIZE as u64;
        let chunk_key = chunk_start;

        // Check cache first
        {
            let mut cache = self.file_cache.lock().unwrap();
            if let Some(file_cache) = cache.get_mut(path) {
                if let Some(cached_chunk) = file_cache.chunks.get_mut(&chunk_key) {
                    // Cache hit!
                    cached_chunk.last_access = SystemTime::now();
                    let chunk_offset = (offset - chunk_start) as usize;
                    let chunk_end = (chunk_offset + size as usize).min(cached_chunk.data.len());

                    log::debug!("Cache HIT for {} at offset {}", path, offset);
                    return Ok(cached_chunk.data[chunk_offset..chunk_end].to_vec());
                }
            }
        }

        // Cache miss - fetch from server
        log::debug!("Cache MISS for {} at offset {}", path, offset);
        let chunk_data = self.api_client.read_file_chunk(path, chunk_start, CHUNK_SIZE)?;

        // Store in cache
        self.cache_chunk(path, chunk_start, chunk_data.clone());

        // Extract requested portion
        let chunk_offset = (offset - chunk_start) as usize;
        let chunk_end = (chunk_offset + size as usize).min(chunk_data.len());
        Ok(chunk_data[chunk_offset..chunk_end].to_vec())
    }

    /// Store a chunk in the cache, evicting old entries if necessary
    fn cache_chunk(&self, path: &str, offset: u64, data: Vec<u8>) {
        let mut cache = self.file_cache.lock().unwrap();

        // Get or create file cache entry
        let file_cache = cache.entry(path.to_string()).or_insert_with(|| FileCache {
            chunks: HashMap::new(),
            total_size: 0,
        });

        // Check if we need to evict old chunks
        while file_cache.total_size + data.len() > MAX_CACHE_SIZE && !file_cache.chunks.is_empty() {
            // Find oldest chunk
            if let Some((&oldest_offset, _)) = file_cache.chunks.iter()
                .min_by_key(|(_, chunk)| chunk.last_access) {
                if let Some(removed) = file_cache.chunks.remove(&oldest_offset) {
                    file_cache.total_size -= removed.data.len();
                    log::debug!("Evicted chunk at offset {} from cache", oldest_offset);
                }
            } else {
                break;
            }
        }

        // Add new chunk
        let chunk_size = data.len();
        file_cache.chunks.insert(offset, CachedChunk {
            data,
            offset,
            last_access: SystemTime::now(),
        });
        file_cache.total_size += chunk_size;
    }

    /// Invalidate cache for a file (called after write operations)
    fn invalidate_cache(&self, path: &str) {
        let mut cache = self.file_cache.lock().unwrap();
        if let Some(removed) = cache.remove(path) {
            log::debug!("Invalidated cache for {} ({} chunks)", path, removed.chunks.len());
        }
    }

    pub fn mount(self, mountpoint: &str) -> anyhow::Result<()> {
        let options = vec![
            MountOption::RW,
            MountOption::FSName("remotefs".to_string()),
            //MountOption::AutoUnmount,
            //MountOption::AllowOther
        ];

        log::info!("Mounting filesystem at {}", mountpoint);
        fuser::mount2(self, mountpoint, &options)?;
        Ok(())
    }
}

impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        log::debug!("lookup(parent={}, name={:?})", parent, name);

        let path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Check if we already have this inode cached
        {
            let path_to_ino = self.path_to_ino.lock().unwrap();
            if let Some(&ino) = path_to_ino.get(&path) {
                if let Some(inode) = self.get_inode(ino) {
                    reply.entry(&TTL, &inode.attr, 0);
                    return;
                }
            }
        }

        // Try to get parent directory listing to find this entry
        let parent_inode = match self.get_inode(parent) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.list_directory(&parent_inode.path) {
            Ok(entries) => {
                for entry in entries {
                    if entry.name == name.to_string_lossy() {
                        let full_path = if parent_inode.path == "/" {
                            format!("/{}", entry.name)
                        } else {
                            format!("{}/{}", parent_inode.path, entry.name)
                        };

                        let ino = self.get_or_create_inode(&full_path, &entry);
                        if let Some(inode) = self.get_inode(ino) {
                            reply.entry(&TTL, &inode.attr, 0);
                            return;
                        }
                    }
                }
                reply.error(ENOENT);
            }
            Err(e) => {
                log::error!("Failed to list directory: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        log::debug!("getattr(ino={})", ino);

        match self.get_inode(ino) {
            Some(inode) => reply.attr(&TTL, &inode.attr),
            None => reply.error(ENOENT),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        log::debug!("setattr(ino={}, mode={:?}, size={:?})", ino, mode, size);

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Handle truncate (size change)
        if let Some(new_size) = size {
            if inode.attr.kind == FileType::RegularFile {
                // Read current file data
                let mut file_data = match self.api_client.read_file(&inode.path) {
                    Ok(data) => data,
                    Err(_) => Vec::new(),
                };

                // Resize the file
                file_data.resize(new_size as usize, 0);

                // Write back to server
                match self.api_client.write_file(&inode.path, &file_data) {
                    Ok(_) => {
                        // Invalidate cache for this file
                        self.invalidate_cache(&inode.path);

                        let mut inodes = self.inodes.lock().unwrap();
                        if let Some(inode) = inodes.get_mut(&ino) {
                            inode.attr.size = new_size;
                            inode.attr.mtime = SystemTime::now();
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to truncate file: {}", e);
                        reply.error(e.errno);
                        return;
                    }
                }
            }
        }

        // Handle mode (permissions) change
        if let Some(new_mode) = mode {
            let mut inodes = self.inodes.lock().unwrap();
            if let Some(inode) = inodes.get_mut(&ino) {
                inode.attr.perm = (new_mode & 0o777) as u16;
                inode.attr.ctime = SystemTime::now();
            }
        }

        // Return updated attributes
        match self.get_inode(ino) {
            Some(inode) => reply.attr(&TTL, &inode.attr),
            None => reply.error(ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        log::debug!("readdir(ino={}, offset={})", ino, offset);

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.list_directory(&inode.path) {
            Ok(entries) => {
                let mut i = offset;

                if i == 0 {
                    if reply.add(ino, i + 1, FileType::Directory, ".") {
                        reply.ok();
                        return;
                    }
                    i += 1;
                }

                if i == 1 {
                    if reply.add(ino, i + 1, FileType::Directory, "..") {
                        reply.ok();
                        return;
                    }
                    i += 1;
                }

                for (_idx, entry) in entries.iter().enumerate().skip((i - 2).max(0) as usize) {
                    let full_path = if inode.path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", inode.path, entry.name)
                    };

                    let entry_ino = self.get_or_create_inode(&full_path, entry);
                    let kind = if entry.is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };

                    if reply.add(entry_ino, i + 1, kind, &entry.name) {
                        break;
                    }
                    i += 1;
                }

                reply.ok();
            }
            Err(e) => {
                log::error!("Failed to list directory: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        log::debug!("open(ino={}, flags={})", ino, _flags);

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Generate a file handle
        let mut next_fh = self.next_fh.lock().unwrap();
        let fh = *next_fh;
        *next_fh += 1;

        // Store the file handle mapping
        let mut file_handles = self.file_handles.lock().unwrap();
        file_handles.insert(fh, inode.path.clone());

        reply.opened(fh, 0);
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        log::debug!("release(fh={})", fh);

        // Remove the file handle
        let mut file_handles = self.file_handles.lock().unwrap();
        file_handles.remove(&fh);

        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        log::debug!("read(ino={}, offset={}, size={})", ino, offset, size);

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Check if offset is beyond file size
        if offset < 0 || offset as u64 >= inode.attr.size {
            reply.data(&[]);
            return;
        }

        // Use cached/chunked reading for better performance with large files
        match self.read_with_cache(&inode.path, offset as u64, size) {
            Ok(data) => {
                reply.data(&data);
            }
            Err(e) => {
                log::error!("Failed to read file: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        log::debug!("write(ino={}, offset={}, size={})", ino, offset, data.len());

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Try to use efficient chunk-based write first
        match self.api_client.write_file_chunk(&inode.path, offset as u64, data) {
            Ok(_) => {
                // Invalidate cache for this file since it's been modified
                self.invalidate_cache(&inode.path);

                // Update inode metadata
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(inode) = inodes.get_mut(&ino) {
                    let new_size = ((offset as u64) + (data.len() as u64)).max(inode.attr.size);
                    inode.attr.size = new_size;
                    inode.attr.mtime = SystemTime::now();
                }

                reply.written(data.len() as u32);
            }
            Err(e) => {
                log::error!("Failed to write file chunk: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        log::debug!("mkdir(parent={}, name={:?})", parent, name);

        let path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.create_directory(&path) {
            Ok(_) => {
                let entry = FileEntry {
                    name: name.to_string_lossy().to_string(),
                    is_dir: true,
                    size: 0,
                    mtime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    ctime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    mode: 0o755,
                };

                let ino = self.get_or_create_inode(&path, &entry);
                if let Some(inode) = self.get_inode(ino) {
                    reply.entry(&TTL, &inode.attr, 0);
                } else {
                    reply.error(libc::EIO);
                }
            }
            Err(e) => {
                log::error!("Failed to create directory: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        log::debug!("unlink(parent={}, name={:?})", parent, name);

        let path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.delete(&path) {
            Ok(_) => {
                // Remove from cache
                let mut path_to_ino = self.path_to_ino.lock().unwrap();
                let mut inodes = self.inodes.lock().unwrap();

                if let Some(ino) = path_to_ino.remove(&path) {
                    inodes.remove(&ino);
                }

                reply.ok();
            }
            Err(e) => {
                log::error!("Failed to delete file: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        log::debug!("rmdir(parent={}, name={:?})", parent, name);

        let path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.delete(&path) {
            Ok(_) => {
                // Remove from cache
                let mut path_to_ino = self.path_to_ino.lock().unwrap();
                let mut inodes = self.inodes.lock().unwrap();

                if let Some(ino) = path_to_ino.remove(&path) {
                    inodes.remove(&ino);
                }

                reply.ok();
            }
            Err(e) => {
                log::error!("Failed to delete directory: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        log::debug!(
            "rename(parent={}, name={:?}, newparent={}, newname={:?})",
            parent, name, newparent, newname
        );

        let from_path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let to_path = match self.path_from_parent_and_name(newparent, newname) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.api_client.rename(&from_path, &to_path) {
            Ok(_) => {
                // Update cache
                let mut path_to_ino = self.path_to_ino.lock().unwrap();
                let mut inodes = self.inodes.lock().unwrap();

                if let Some(ino) = path_to_ino.remove(&from_path) {
                    path_to_ino.insert(to_path.clone(), ino);
                    if let Some(inode) = inodes.get_mut(&ino) {
                        inode.path = to_path;
                    }
                }

                reply.ok();
            }
            Err(e) => {
                log::error!("Failed to rename: {}", e);
                reply.error(e.errno);
            }
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        log::debug!("create(parent={}, name={:?})", parent, name);

        let path = match self.path_from_parent_and_name(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Create empty file on server
        match self.api_client.write_file(&path, &[]) {
            Ok(_) => {
                let entry = FileEntry {
                    name: name.to_string_lossy().to_string(),
                    is_dir: false,
                    size: 0,
                    mtime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    ctime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    mode: 0o644,
                };

                let ino = self.get_or_create_inode(&path, &entry);
                if let Some(inode) = self.get_inode(ino) {
                    let mut next_fh = self.next_fh.lock().unwrap();
                    let fh = *next_fh;
                    *next_fh += 1;

                    reply.created(&TTL, &inode.attr, 0, fh, 0);
                } else {
                    reply.error(libc::EIO);
                }
            }
            Err(e) => {
                log::error!("Failed to create file: {}", e);
                reply.error(e.errno);
            }
        }
    }
}
