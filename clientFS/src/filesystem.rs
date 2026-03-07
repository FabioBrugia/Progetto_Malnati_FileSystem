use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyWrite, Request, Session, SessionUnmounter,
};
use libc::ENOENT;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::api_client::{ApiClient, FileEntry};
use crate::cache::{CacheManager, CHUNK_SIZE, METADATA_CACHE_TTL};

/// TTL restituito al kernel FUSE per le entry.
const FUSE_TTL: Duration = Duration::from_secs(1);

// ─── INode ───────────────────────────────────────────────────────────

/// Rappresenta un inode nel filesystem virtuale.
#[derive(Debug, Clone)]
struct INode {
    #[allow(dead_code)]
    ino: u64,
    path: String,
    attr: FileAttr,
    /// Timestamp per tracciare quando l'inode è stato cachato
    cached_at: Instant,
}

impl INode {
    fn is_metadata_expired(&self) -> bool {
        self.cached_at.elapsed() > METADATA_CACHE_TTL
    }
}

// ─── Inode Table ─────────────────────────────────────────────────────

/// Tabella degli inode: gestisce il mapping path ↔ inode number.
struct InodeTable {
    inodes: HashMap<u64, INode>,
    path_to_ino: HashMap<String, u64>,
    next_ino: u64,
}

impl InodeTable {
    fn new() -> Self {
        let mut table = Self {
            inodes: HashMap::new(),
            path_to_ino: HashMap::new(),
            next_ino: 2,
        };

        // Crea l'inode root (ino=1)
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
            cached_at: Instant::now(),
        };

        table.inodes.insert(1, root_inode);
        table.path_to_ino.insert("/".to_string(), 1);

        table
    }

    /// Ottiene o crea un inode per il path dato, aggiornando i metadati.
    fn get_or_create(&mut self, path: &str, entry: &FileEntry) -> u64 {
        if let Some(&ino) = self.path_to_ino.get(path) {
            // Aggiorna l'inode esistente con i nuovi metadati (refresh)
            if let Some(inode) = self.inodes.get_mut(&ino) {
                inode.attr.size = entry.size;
                inode.attr.mtime = UNIX_EPOCH + Duration::from_secs_f64(entry.mtime);
                inode.attr.ctime = UNIX_EPOCH + Duration::from_secs_f64(entry.ctime);
                inode.cached_at = Instant::now();
            }
            return ino;
        }

        let ino = self.next_ino;
        self.next_ino += 1;

        let attr = FileAttr {
            ino,
            size: entry.size,
            blocks: (entry.size + 511) / 512,
            atime: UNIX_EPOCH + Duration::from_secs_f64(entry.mtime),
            mtime: UNIX_EPOCH + Duration::from_secs_f64(entry.mtime),
            ctime: UNIX_EPOCH + Duration::from_secs_f64(entry.ctime),
            crtime: UNIX_EPOCH + Duration::from_secs_f64(entry.ctime),
            kind: if entry.is_dir { FileType::Directory } else { FileType::RegularFile },
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
            cached_at: Instant::now(),
        };

        self.inodes.insert(ino, inode);
        self.path_to_ino.insert(path.to_string(), ino);
        ino
    }

    fn get(&self, ino: u64) -> Option<&INode> {
        self.inodes.get(&ino)
    }

    fn get_cloned(&self, ino: u64) -> Option<INode> {
        self.inodes.get(&ino).cloned()
    }

    fn get_mut(&mut self, ino: u64) -> Option<&mut INode> {
        self.inodes.get_mut(&ino)
    }

    /// Calcola il path figlio a partire dall'inode parent e dal nome.
    fn child_path(&self, parent: u64, name: &OsStr) -> Option<String> {
        let parent_inode = self.inodes.get(&parent)?;
        let name_str = name.to_str()?;

        let path = if parent_inode.path == "/" {
            format!("/{}", name_str)
        } else {
            format!("{}/{}", parent_inode.path, name_str)
        };
        Some(path)
    }

    /// Rimuove un inode dato il suo path.
    fn remove_by_path(&mut self, path: &str) {
        if let Some(ino) = self.path_to_ino.remove(path) {
            self.inodes.remove(&ino);
        }
    }

    /// Rinomina un inode da un path a un altro.
    fn rename(&mut self, from: &str, to: &str) {
        if let Some(ino) = self.path_to_ino.remove(from) {
            self.path_to_ino.insert(to.to_string(), ino);
            if let Some(inode) = self.inodes.get_mut(&ino) {
                inode.path = to.to_string();
                inode.cached_at = Instant::now();
            }
        }
    }

    /// Invalida i metadati di un inode (forza refresh al prossimo accesso).
    fn invalidate_metadata(&mut self, path: &str) {
        if let Some(&ino) = self.path_to_ino.get(path) {
            if let Some(inode) = self.inodes.get_mut(&ino) {
                inode.cached_at = Instant::now() - METADATA_CACHE_TTL - Duration::from_secs(1);
                log::debug!("Invalidated metadata cache for {}", path);
            }
        }
    }
}

// ─── File Handle Table ───────────────────────────────────────────────

/// Gestisce i file handle aperti.
struct FileHandleTable {
    handles: HashMap<u64, String>,
    next_fh: u64,
}

impl FileHandleTable {
    fn new() -> Self {
        Self {
            handles: HashMap::new(),
            next_fh: 1,
        }
    }

    fn open(&mut self, path: String) -> u64 {
        let fh = self.next_fh;
        self.next_fh += 1;
        self.handles.insert(fh, path);
        fh
    }

    fn close(&mut self, fh: u64) {
        self.handles.remove(&fh);
    }
}

// ─── Path Lock Manager ──────────────────────────────────────────────

/// Gestisce lock per singolo path per serializzare operazioni mutanti
/// solo quando insistono sullo stesso target logico.
struct PathLockManager {
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl PathLockManager {
    fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    fn get_lock(&self, path: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap();
        locks
            .entry(path.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

// ─── RemoteFS ────────────────────────────────────────────────────────

/// Filesystem remoto FUSE.
///
/// Implementa il trait `Filesystem` di fuser, delegando le operazioni
/// di rete all'`ApiClient` e usando il `CacheManager` per la cache locale.
pub struct RemoteFS {
    api_client: Arc<ApiClient>,
    inode_table: Arc<RwLock<InodeTable>>,
    file_handles: Arc<Mutex<FileHandleTable>>,
    cache: Arc<Mutex<CacheManager>>,
    path_locks: Arc<PathLockManager>,
}

impl RemoteFS {
    pub fn new(api_client: ApiClient) -> Self {
        Self {
            api_client: Arc::new(api_client),
            inode_table: Arc::new(RwLock::new(InodeTable::new())),
            file_handles: Arc::new(Mutex::new(FileHandleTable::new())),
            cache: Arc::new(Mutex::new(CacheManager::new())),
            path_locks: Arc::new(PathLockManager::new()),
        }
    }

    /// Invalida tutte le cache per un path (write-through).
    fn invalidate_all_for_path(&self, path: &str) {
        self.cache.lock().unwrap().invalidate_all_for_path(path);
        self.inode_table.write().unwrap().invalidate_metadata(path);
    }

    /// Monta il filesystem al mountpoint specificato.
    ///
    /// Salva un `SessionUnmounter` nell'`Arc<Mutex>` fornito **prima** di
    /// avviare il loop FUSE, così un signal handler può smontare il
    /// filesystem in modo pulito (graceful shutdown).
    /// La funzione blocca finché la sessione FUSE non viene terminata.
    pub fn mount(
        self,
        mountpoint: &Path,
        unmounter_slot: Arc<Mutex<Option<SessionUnmounter>>>,
    ) -> anyhow::Result<()> {
        let mut options = vec![
            MountOption::FSName("remoteFS".to_string()),
            MountOption::AutoUnmount,
        ];

        #[cfg(target_os = "linux")]
        {
            options.push(MountOption::AllowOther);
            options.push(MountOption::DefaultPermissions);
        }

        #[cfg(target_os = "macos")]
        {
            options.push(MountOption::RW);
        }

        log::info!("Mounting filesystem at {}", mountpoint.display());

        let mut session = Session::new(self, mountpoint, &options)
            .map_err(|e| anyhow::anyhow!("Failed to create FUSE session: {}", e))?;

        // Salva l'unmounter PRIMA di avviare il loop, così il signal handler
        // può trovarlo immediatamente
        {
            let mut guard = unmounter_slot.lock().unwrap();
            *guard = Some(session.unmount_callable());
        }
        log::info!("SessionUnmounter pronto per il graceful shutdown");

        // Esegue il loop della sessione FUSE (bloccante)
        session.run()
            .map_err(|e| anyhow::anyhow!("FUSE session error: {}", e))?;

        log::info!("Sessione FUSE terminata.");
        Ok(())
    }
}

// ─── Implementazione trait Filesystem ────────────────────────────────

impl Filesystem for RemoteFS {
    fn destroy(&mut self) {
        log::info!("Filesystem in fase di smontaggio — cleanup in corso...");

        // Flush della cache
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
            log::info!("Cache svuotata.");
        }

        // Chiudi tutti i file handle aperti
        if let Ok(mut handles) = self.file_handles.lock() {
            let count = handles.handles.len();
            handles.handles.clear();
            handles.next_fh = 1;
            log::info!("Chiusi {} file handle aperti.", count);
        }

        // Pulisci la tabella degli inode
        if let Ok(mut table) = self.inode_table.write() {
            let count = table.inodes.len();
            table.inodes.clear();
            table.path_to_ino.clear();
            log::info!("Rilasciati {} inode.", count);
        }

        log::info!("Cleanup completato. Filesystem smontato correttamente.");
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        log::debug!("lookup(parent={}, name={:?})", parent, name);

        let path = match self.inode_table.read().unwrap().child_path(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Controlla se l'inode è in cache e non scaduto
        {
            let table = self.inode_table.read().unwrap();
            if let Some(&ino) = table.path_to_ino.get(&path) {
                if let Some(inode) = table.get(ino) {
                    if !inode.is_metadata_expired() {
                        log::debug!("Metadata cache HIT for {} (TTL valid)", path);
                        reply.entry(&FUSE_TTL, &inode.attr, 0);
                        return;
                    } else {
                        log::debug!("Metadata cache EXPIRED for {}", path);
                    }
                }
            }
        }

        // Ottieni il path del parent per il listing
        let parent_path = match self.inode_table.read().unwrap().get_cloned(parent) {
            Some(inode) => inode.path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let cached_entries = self
            .cache
            .try_lock()
            .ok()
            .and_then(|mut cache| cache.get_cached_directory(&parent_path));

        let entries = if let Some(cached_entries) = cached_entries {
            cached_entries
        } else {
            match self.api_client.list_directory(&parent_path) {
                Ok(fresh_entries) => {
                    if let Ok(mut cache) = self.cache.try_lock() {
                        cache.store_directory_listing(&parent_path, fresh_entries.clone());
                    }
                    fresh_entries
                }
                Err(e) => {
                    log::error!("Failed to list directory: {}", e);
                    reply.error(e.errno);
                    return;
                }
            }
        };

        // Cerca l'entry nel listing della directory parent
        {
            for entry in entries {
                if entry.name == name.to_string_lossy() {
                    let full_path = if parent_path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", parent_path, entry.name)
                    };

                    let ino = self.inode_table.write().unwrap().get_or_create(&full_path, &entry);
                    if let Some(inode) = self.inode_table.read().unwrap().get_cloned(ino) {
                        reply.entry(&FUSE_TTL, &inode.attr, 0);
                        return;
                    }
                }
            }
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        log::debug!("getattr(ino={})", ino);

        // Pulizia cache periodica sulla root
        if ino == 1 {
            if let Ok(mut cache) = self.cache.try_lock() {
                cache.cleanup_expired();
            }
        }

        match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => {
                if inode.is_metadata_expired() && ino != 1 {
                    log::debug!(
                        "Metadata expired for ino={}, returning cached (will refresh on next lookup)",
                        ino
                    );
                }
                reply.attr(&FUSE_TTL, &inode.attr);
            }
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

        let inode = match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&inode.path);
        let _path_guard = path_lock.lock().unwrap();

        // Handle truncate (size change) — WRITE-THROUGH
        if let Some(new_size) = size {
            if inode.attr.kind == FileType::RegularFile {
                let mut file_data = match self.api_client.read_file(&inode.path) {
                    Ok(data) => data,
                    Err(_) => Vec::new(),
                };

                file_data.resize(new_size as usize, 0);

                match self.api_client.write_file(&inode.path, &file_data) {
                    Ok(_) => {
                        self.invalidate_all_for_path(&inode.path);

                        let mut table = self.inode_table.write().unwrap();
                        if let Some(node) = table.get_mut(ino) {
                            node.attr.size = new_size;
                            node.attr.mtime = SystemTime::now();
                            node.cached_at = Instant::now();
                        }
                        log::info!("Write-through: truncated {} to {} bytes", inode.path, new_size);
                    }
                    Err(e) => {
                        log::error!("Failed to truncate file: {}", e);
                        reply.error(e.errno);
                        return;
                    }
                }
            }
        }

        // Handle mode (permissions) change — WRITE-THROUGH to server
        if let Some(new_mode) = mode {
            match self.api_client.set_attrs(&inode.path, Some(new_mode & 0o777)) {
                Ok(_) => {
                    // Invalida tutte le cache associate a questo path per garantire
                    // consistenza tra metadati locali e stato remoto.
                    self.invalidate_all_for_path(&inode.path);

                    let mut table = self.inode_table.write().unwrap();
                    if let Some(node) = table.get_mut(ino) {
                        node.attr.perm = (new_mode & 0o777) as u16;
                        node.attr.ctime = SystemTime::now();
                        node.cached_at = Instant::now();
                    }
                    log::info!(
                        "Write-through: updated permissions for {} to {:o}",
                        inode.path,
                        new_mode & 0o777
                    );
                }
                Err(e) => {
                    log::error!("Failed to set attrs for {}: {}", inode.path, e);
                    reply.error(e.errno);
                    return;
                }
            }
        }

        // Return updated attributes
        match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => reply.attr(&FUSE_TTL, &inode.attr),
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

        let inode = match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let cached_entries = self
            .cache
            .try_lock()
            .ok()
            .and_then(|mut cache| cache.get_cached_directory(&inode.path));

        let entries = if let Some(cached_entries) = cached_entries {
            cached_entries
        } else {
            match self.api_client.list_directory(&inode.path) {
                Ok(fresh_entries) => {
                    if let Ok(mut cache) = self.cache.try_lock() {
                        cache.store_directory_listing(&inode.path, fresh_entries.clone());
                    }
                    fresh_entries
                }
                Err(e) => {
                    log::error!("Failed to list directory: {}", e);
                    reply.error(e.errno);
                    return;
                }
            }
        };

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

        let mut table = self.inode_table.write().unwrap();
        for entry in entries.iter().skip((i - 2).max(0) as usize) {
            let full_path = if inode.path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", inode.path, entry.name)
            };

            let entry_ino = table.get_or_create(&full_path, entry);
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

    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        log::debug!("open(ino={}, flags={})", ino, _flags);

        let path = match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => inode.path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let fh = self.file_handles.lock().unwrap().open(path);
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
        self.file_handles.lock().unwrap().close(fh);
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

        let inode = match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if offset < 0 || offset as u64 >= inode.attr.size {
            reply.data(&[]);
            return;
        }

        let offset_u64 = offset as u64;
        let cached_data = self
            .cache
            .try_lock()
            .ok()
            .and_then(|mut cache| cache.read_from_cache(&inode.path, offset_u64, size));

        if let Some(cached_data) = cached_data {
            reply.data(&cached_data);
            return;
        }

        let chunk_start = (offset_u64 / CHUNK_SIZE as u64) * CHUNK_SIZE as u64;
        match self.api_client.read_file_chunk(&inode.path, chunk_start, CHUNK_SIZE) {
            Ok(chunk_data) => {
                if let Ok(mut cache) = self.cache.try_lock() {
                    cache.store_file_chunk(&inode.path, chunk_start, chunk_data.clone());
                }

                let chunk_offset = (offset_u64 - chunk_start) as usize;
                let chunk_end = (chunk_offset + size as usize).min(chunk_data.len());
                reply.data(&chunk_data[chunk_offset..chunk_end]);
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

        let inode = match self.inode_table.read().unwrap().get_cloned(ino) {
            Some(inode) => inode,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&inode.path);
        let _path_guard = path_lock.lock().unwrap();

        // WRITE-THROUGH: scrivi immediatamente sul server
        match self.api_client.write_file_chunk(&inode.path, offset as u64, data) {
            Ok(_) => {
                self.invalidate_all_for_path(&inode.path);

                let mut table = self.inode_table.write().unwrap();
                if let Some(node) = table.get_mut(ino) {
                    let new_size = ((offset as u64) + (data.len() as u64)).max(node.attr.size);
                    node.attr.size = new_size;
                    node.attr.mtime = SystemTime::now();
                    node.cached_at = Instant::now();
                }

                log::debug!(
                    "Write-through: wrote {} bytes to {} at offset {}",
                    data.len(), inode.path, offset
                );
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

        let path = match self.inode_table.read().unwrap().child_path(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&path);
        let _path_guard = path_lock.lock().unwrap();

        match self.api_client.create_directory(&path) {
            Ok(_) => {
                self.cache.lock().unwrap().invalidate_directory_cache(&path);

                let entry = FileEntry {
                    name: name.to_string_lossy().to_string(),
                    is_dir: true,
                    size: 0,
                    mtime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    ctime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    mode: 0o755,
                };

                let ino = self.inode_table.write().unwrap().get_or_create(&path, &entry);
                if let Some(inode) = self.inode_table.read().unwrap().get_cloned(ino) {
                    log::info!("Write-through: created directory {}", path);
                    reply.entry(&FUSE_TTL, &inode.attr, 0);
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

        let path = match self.inode_table.read().unwrap().child_path(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&path);
        let _path_guard = path_lock.lock().unwrap();

        match self.api_client.delete(&path) {
            Ok(_) => {
                self.invalidate_all_for_path(&path);
                self.inode_table.write().unwrap().remove_by_path(&path);
                log::info!("Write-through: deleted file {}", path);
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

        let path = match self.inode_table.read().unwrap().child_path(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&path);
        let _path_guard = path_lock.lock().unwrap();

        match self.api_client.delete(&path) {
            Ok(_) => {
                self.invalidate_all_for_path(&path);
                self.inode_table.write().unwrap().remove_by_path(&path);
                log::info!("Write-through: deleted directory {}", path);
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

        let (from_path, to_path) = {
            let table = self.inode_table.read().unwrap();
            let from = match table.child_path(parent, name) {
                Some(p) => p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };
            let to = match table.child_path(newparent, newname) {
                Some(p) => p,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };
            (from, to)
        };

        if from_path == to_path {
            log::debug!("rename no-op: source and destination are the same ({})", from_path);
            reply.ok();
            return;
        }

        let (first_lock_path, second_lock_path) = if from_path <= to_path {
            (from_path.as_str(), to_path.as_str())
        } else {
            (to_path.as_str(), from_path.as_str())
        };
        let first_lock = self.path_locks.get_lock(first_lock_path);
        let second_lock = self.path_locks.get_lock(second_lock_path);
        let _first_guard = first_lock.lock().unwrap();
        let _second_guard = second_lock.lock().unwrap();

        match self.api_client.rename(&from_path, &to_path) {
            Ok(_) => {
                self.invalidate_all_for_path(&from_path);

                if let Some((parent, _)) = to_path.rsplit_once('/') {
                    let to_parent = if parent.is_empty() { "/" } else { parent };
                    self.cache.lock().unwrap().invalidate_directory_cache(to_parent);
                }

                self.invalidate_all_for_path(&to_path);
                self.inode_table.write().unwrap().rename(&from_path, &to_path);
                log::info!("Write-through: renamed {} -> {}", from_path, to_path);
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

        let path = match self.inode_table.read().unwrap().child_path(parent, name) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path_lock = self.path_locks.get_lock(&path);
        let _path_guard = path_lock.lock().unwrap();

        match self.api_client.write_file(&path, &[]) {
            Ok(_) => {
                self.cache.lock().unwrap().invalidate_directory_cache(&path);

                let entry = FileEntry {
                    name: name.to_string_lossy().to_string(),
                    is_dir: false,
                    size: 0,
                    mtime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    ctime: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),
                    mode: 0o644,
                };

                let ino = self.inode_table.write().unwrap().get_or_create(&path, &entry);
                let inode = self.inode_table.read().unwrap().get_cloned(ino);

                if let Some(inode) = inode {
                    let fh = self.file_handles.lock().unwrap().open(path.clone());
                    log::info!("Write-through: created file {}", path);
                    reply.created(&FUSE_TTL, &inode.attr, 0, fh, 0);
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
