use fuse::{
    FileAttr, FileType, Filesystem, Request, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEntry, ReplyWrite, ReplyCreate, ReplyEmpty
};
use libc::{ENOENT, ENOTDIR, EISDIR, EIO};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use time::Timespec;
use tokio::runtime::Runtime;
use crate::remote;

pub struct RemoteFS {
    remote_client: Arc<remote::Client>,
    cache_enabled: bool,
    rt: Arc<Runtime>,
    inode_map: Arc<Mutex<HashMap<u64, String>>>, // inode -> path
    path_map: Arc<Mutex<HashMap<String, u64>>>,  // path -> inode
    next_inode: Arc<Mutex<u64>>,
}

impl RemoteFS {
    pub fn new(remote_client: remote::Client, cache_enabled: bool) -> Self {
        let rt = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
        let mut inode_map = HashMap::new();
        let mut path_map = HashMap::new();

        // Root directory
        inode_map.insert(1, "/".to_string());
        path_map.insert("/".to_string(), 1);

        Self {
            remote_client: Arc::new(remote_client),
            cache_enabled,
            rt,
            inode_map: Arc::new(Mutex::new(inode_map)),
            path_map: Arc::new(Mutex::new(path_map)),
            next_inode: Arc::new(Mutex::new(2)),
        }
    }

    pub fn mount_and_run(self, mount_point: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let options = [
            OsString::from("-o"), 
            OsString::from("fsname=remote-fs"), 
            OsString::from("-o"), 
            OsString::from("auto_unmount")
        ];
        let options_refs: Vec<&OsStr> = options.iter().map(|s| s.as_os_str()).collect();
        fuse::mount(self, &mount_point, &options_refs)?;
        Ok(())
    }

    fn get_or_create_inode(&self, path: &str) -> u64 {
        let mut path_map = self.path_map.lock().unwrap();
        let mut inode_map = self.inode_map.lock().unwrap();

        if let Some(&ino) = path_map.get(path) {
            return ino;
        }

        let mut next_inode = self.next_inode.lock().unwrap();
        let ino = *next_inode;
        *next_inode += 1;

        path_map.insert(path.to_string(), ino);
        inode_map.insert(ino, path.to_string());

        ino
    }

    fn get_path(&self, ino: u64) -> Option<String> {
        self.inode_map.lock().unwrap().get(&ino).cloned()
    }

    fn system_time_to_timespec(st: std::time::SystemTime) -> Timespec {
        match st.duration_since(std::time::UNIX_EPOCH) {
            Ok(dur) => Timespec::new(dur.as_secs() as i64, dur.subsec_nanos() as i32),
            Err(_) => Timespec::new(0, 0),
        }
    }
}

impl Filesystem for RemoteFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let parent_path = match self.get_path(parent) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let child_path = if parent_path == "/" {
            format!("/{}", name.to_string_lossy())
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.get_file_info(&child_path).await
        });

        match result {
            Ok(file_info) => {
                let ino = self.get_or_create_inode(&child_path);
                let attr = FileAttr {
                    ino,
                    size: file_info.size,
                    blocks: (file_info.size + 511) / 512,
                    atime: Self::system_time_to_timespec(file_info.modified),
                    mtime: Self::system_time_to_timespec(file_info.modified),
                    ctime: Self::system_time_to_timespec(file_info.modified),
                    crtime: Self::system_time_to_timespec(file_info.modified),
                    kind: if file_info.is_dir { FileType::Directory } else { FileType::RegularFile },
                    perm: if file_info.is_dir { 0o755 } else { 0o644 },
                    nlink: if file_info.is_dir { 2 } else { 1 },
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    flags: 0,
                };
                reply.entry(&Timespec::new(1, 0), &attr, 0);
            },
            Err(_) => reply.error(ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            // Root directory
            let attr = FileAttr {
                ino: 1,
                size: 0,
                blocks: 0,
                atime: Timespec::new(0, 0),
                mtime: Timespec::new(0, 0),
                ctime: Timespec::new(0, 0),
                crtime: Timespec::new(0, 0),
                kind: FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                flags: 0,
            };
            reply.attr(&Timespec::new(1, 0), &attr);
            return;
        }

        let path = match self.get_path(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.get_file_info(&path).await
        });

        match result {
            Ok(file_info) => {
                let attr = FileAttr {
                    ino,
                    size: file_info.size,
                    blocks: (file_info.size + 511) / 512,
                    atime: Self::system_time_to_timespec(file_info.modified),
                    mtime: Self::system_time_to_timespec(file_info.modified),
                    ctime: Self::system_time_to_timespec(file_info.modified),
                    crtime: Self::system_time_to_timespec(file_info.modified),
                    kind: if file_info.is_dir { FileType::Directory } else { FileType::RegularFile },
                    perm: if file_info.is_dir { 0o755 } else { 0o644 },
                    nlink: if file_info.is_dir { 2 } else { 1 },
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    flags: 0,
                };
                reply.attr(&Timespec::new(1, 0), &attr);
            },
            Err(_) => reply.error(ENOENT),
        }
    }

    fn read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, size: u32, reply: ReplyData) {
        let path = match self.get_path(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.read_file(&path, offset as u64, size).await
        });

        match result {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(EIO),
        }
    }

    fn write(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, data: &[u8], _flags: u32, reply: ReplyWrite) {
        let path = match self.get_path(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();
        let data_copy = data.to_vec();

        let result = rt.block_on(async {
            client.write_file(&path, offset as u64, &data_copy).await
        });

        match result {
            Ok(written) => reply.written(written),
            Err(_) => reply.error(EIO),
        }
    }

    fn readdir(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        let path = match self.get_path(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if offset == 0 {
            reply.add(ino, 0, FileType::Directory, ".");
            reply.add(ino, 1, FileType::Directory, "..");
        }

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.list_directory(&path).await
        });

        match result {
            Ok(entries) => {
                for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                    let child_path = if path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", path, entry.name)
                    };
                    let child_ino = self.get_or_create_inode(&child_path);
                    let file_type = if entry.is_dir { FileType::Directory } else { FileType::RegularFile };
                    reply.add(child_ino, (i + 2) as i64, file_type, &entry.name);
                }
                reply.ok();
            },
            Err(_) => reply.error(EIO),
        }
    }

    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, _mode: u32, reply: ReplyEntry) {
        let parent_path = match self.get_path(parent) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let new_dir_path = if parent_path == "/" {
            format!("/{}", name.to_string_lossy())
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.create_directory(&new_dir_path).await
        });

        match result {
            Ok(_) => {
                let ino = self.get_or_create_inode(&new_dir_path);
                let attr = FileAttr {
                    ino,
                    size: 0,
                    blocks: 0,
                    atime: Timespec::new(0, 0),
                    mtime: Timespec::new(0, 0),
                    ctime: Timespec::new(0, 0),
                    crtime: Timespec::new(0, 0),
                    kind: FileType::Directory,
                    perm: 0o755,
                    nlink: 2,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    flags: 0,
                };
                reply.entry(&Timespec::new(1, 0), &attr, 0);
            },
            Err(_) => reply.error(EIO),
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let parent_path = match self.get_path(parent) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let file_path = if parent_path == "/" {
            format!("/{}", name.to_string_lossy())
        } else {
            format!("{}/{}", parent_path, name.to_string_lossy())
        };

        let client = self.remote_client.clone();
        let rt = self.rt.clone();

        let result = rt.block_on(async {
            client.delete_file(&file_path).await
        });

        match result {
            Ok(_) => reply.ok(),
            Err(_) => reply.error(EIO),
        }
    }
}
