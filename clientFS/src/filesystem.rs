use anyhow::Result;
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyWrite, Request,
};
use libc::ENOENT;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::api_client::{ApiClient, FileEntry};

const TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct INode {
    #[allow(dead_code)]
    ino: u64,
    path: String,
    attr: FileAttr,
}

pub struct RemoteFS {
    api_client: Arc<ApiClient>,
    inodes: Arc<Mutex<HashMap<u64, INode>>>,
    path_to_ino: Arc<Mutex<HashMap<String, u64>>>,
    next_ino: Arc<Mutex<u64>>,
    #[allow(dead_code)]
    file_handles: Arc<Mutex<HashMap<u64, Vec<u8>>>>,
    next_fh: Arc<Mutex<u64>>,
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

    pub fn mount(self, mountpoint: &str) -> Result<()> {
        let options = vec![
            MountOption::RW,
            MountOption::FSName("remotefs".to_string()),
        ];

        log::info!("Mounting filesystem at {}", mountpoint);
        fuser::mount2(self, mountpoint, &options)?;
        Ok(())
    }
}

impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
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
                reply.error(ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        log::debug!("getattr(ino={})", ino);

        match self.get_inode(ino) {
            Some(inode) => reply.attr(&TTL, &inode.attr),
            None => reply.error(ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
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
                reply.error(libc::EIO);
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
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

        match self.api_client.read_file(&inode.path) {
            Ok(data) => {
                let start = offset as usize;
                let end = (start + size as usize).min(data.len());

                if start >= data.len() {
                    reply.data(&[]);
                } else {
                    reply.data(&data[start..end]);
                }
            }
            Err(e) => {
                log::error!("Failed to read file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        log::debug!("write(ino={}, fh={}, offset={}, size={})", ino, fh, offset, data.len());

        let inode = match self.get_inode(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Read existing file data
        let mut file_data = match self.api_client.read_file(&inode.path) {
            Ok(data) => data,
            Err(_) => Vec::new(), // New file
        };

        // Expand file if necessary
        let end_offset = (offset as usize) + data.len();
        if end_offset > file_data.len() {
            file_data.resize(end_offset, 0);
        }

        // Write data at offset
        file_data[offset as usize..end_offset].copy_from_slice(data);

        // Write back to server
        match self.api_client.write_file(&inode.path, &file_data) {
            Ok(_) => {
                // Update inode size
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(inode) = inodes.get_mut(&ino) {
                    inode.attr.size = file_data.len() as u64;
                    inode.attr.mtime = SystemTime::now();
                }
                reply.written(data.len() as u32);
            }
            Err(e) => {
                log::error!("Failed to write file: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
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
                reply.error(libc::EIO);
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
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
                reply.error(libc::EIO);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
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
                reply.error(libc::EIO);
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: fuser::ReplyEmpty,
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
                reply.error(libc::EIO);
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
        reply: fuser::ReplyCreate,
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
                reply.error(libc::EIO);
            }
        }
    }
}

