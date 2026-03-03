# Remote File System (Rust)

Un filesystem remoto implementato in Rust che presenta un mount point locale, rispecchiando la struttura di un file system ospitato su un server remoto.

## Caratteristiche

- ✅ Interfaccia filesystem locale che interagisce con storage remoto
- ✅ Operazioni standard sui file (lettura, scrittura, creazione, eliminazione, rinomina)
- ✅ Supporto completo per Linux usando FUSE
- ✅ Server RESTful implementato in Rust/Actix Web
- ✅ Client FUSE implementato in Rust
- ✅ **Mapping fine degli errori HTTP → POSIX** per messaggi di errore chiari e comportamento corretto
- ✅ **Cache locale con scadenza (TTL)** per metadati, directory listing e dati file
- ✅ **Politica write-through** con invalidazione automatica della cache su modifiche
- ✅ **Modalità daemon** per esecuzione in background
- ✅ **Recovery automatico mount point** da stati zombie (ENOTCONN)
- ✅ Logging dettagliato con livelli di debug configurabili

## Prerequisiti

### Per il server Rust:
```bash
# Toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# (Opzionale) aggiorna
rustup update
```

### Per il client Rust:
```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# FUSE library (Linux)
sudo apt-get install fuse3 libfuse3-dev  # Debian/Ubuntu
sudo dnf install fuse3 fuse3-devel       # Fedora
sudo pacman -S fuse3                     # Arch Linux

# Build tools
sudo apt-get install build-essential     # Debian/Ubuntu
```

## Installazione

### 1. Compilare il server Rust:
```bash
cargo build --bin server
```

### 2. Compilare il client Rust:
```bash
cd clientFS
cargo build --release
```

## Utilizzo

### 1. Avviare il server:
```bash
cargo run --bin server
```

Il server parte su `http://0.0.0.0:8080` e utilizza `/tmp/remote_fs_test` come storage locale (creato automaticamente).

### 2. Creare un mount point e avviare il client:
```bash
# Creare la directory di mount
mkdir -p /tmp/remotefs

# Avviare il client (in un altro terminale)
cd clientFS
cargo run --release -- --server http://localhost:8080 --mountpoint /tmp/remotefs --verbose
```

### 2b. Avviare il client in modalità daemon (background):
```bash
cd clientFS
cargo run --release -- --server http://localhost:8080 --mountpoint /tmp/remotefs --daemon

# Per fermare il daemon:
cargo run --release -- --mountpoint /tmp/remotefs --stop
```

### 3. Utilizzare il filesystem:
```bash
# Aprire un nuovo terminale e utilizzare il filesystem normalmente
cd /tmp/remotefs
ls -la
cat test/hello.txt
echo "Hello World" > newfile.txt
mkdir newdir
```

### 4. Smontare il filesystem:
Premere `Ctrl+C` nel terminale dove è in esecuzione il client.

## API del Server

Il server espone le seguenti API RESTful:

- `GET /list/<path>` – Lista il contenuto di una directory
- `GET /files/<path>` – Legge il contenuto di un file
- `PUT /files/<path>` – Scrive il contenuto di un file
- `POST /mkdir/<path>` – Crea una directory
- `DELETE /files/<path>` – Elimina un file o directory
- `POST /rename` – Rinomina o sposta un file/directory
- `GET /health` – Health check

## Architettura

```
┌─────────────┐          ┌──────────────┐          ┌─────────────┐
│   Sistema   │  FUSE    │ Client Rust  │   HTTP   │   Server    │
│ Operativo   │ ◄──────► │   (FUSE)     │ ◄──────► │   Rust/Actix│
│             │          │              │          │             │
└─────────────┘          └──────────────┘          └─────────────┘
                                                           │
                                                           ▼
                                                    ┌─────────────┐
                                                    │   File      │
                                                    │   Storage   │
                                                    └─────────────┘
```

## Operazioni Supportate

- ✅ Lettura file
- ✅ Scrittura file
- ✅ Creazione file
- ✅ Eliminazione file
- ✅ Creazione directory
- ✅ Eliminazione directory
- ✅ Rinomina/spostamento
- ✅ Listing directory
- ✅ Attributi file (dimensione, timestamp, permessi)

## Gestione Errori HTTP → POSIX

Il client implementa un mapping fine degli errori HTTP ai codici di errore POSIX per un comportamento più robusto e messaggi di errore chiari:

| HTTP Status | Errno POSIX | Significato |
|-------------|-------------|-------------|
| 400 | `EINVAL` | Parametri non validi |
| 401, 403 | `EACCES` | Permesso negato |
| **404** | **`ENOENT`** | **File o directory non trovata** |
| 405 | `ENOSYS` | Operazione non supportata |
| 409 | `EEXIST` | Risorsa già esistente |
| 413 | `EFBIG` | File troppo grande |
| **500** | **`EIO`** | **Errore I/O del server** |
| 503 | `EAGAIN` | Servizio non disponibile |
| 507 | `ENOSPC` | Spazio insufficiente |
| Timeout | `ETIMEDOUT` | Timeout connessione |
| Network | `EHOSTUNREACH` | Server non raggiungibile |

### Esempi:
```bash
# File non trovato → ENOENT
$ cat /tmp/remotefs/nonexistent.txt
cat: nonexistent.txt: File o directory non esistente

# Directory già esistente → EEXIST
$ mkdir /tmp/remotefs/existing
$ mkdir /tmp/remotefs/existing
mkdir: existing: File già esistente
```

Per maggiori dettagli, vedere: [ERROR_MAPPING_IMPLEMENTATION.md](ERROR_MAPPING_IMPLEMENTATION.md)

## Cache Locale con TTL

Il client implementa una cache intelligente con scadenza (TTL) per ottimizzare le prestazioni e ridurre le richieste al server:

### Livelli di Cache

| Tipo Cache | TTL | Descrizione |
|------------|-----|-------------|
| **Metadati** | 5 secondi | Attributi file (dimensione, timestamp, permessi) |
| **Directory** | 3 secondi | Listing delle directory |
| **Dati File** | 30 secondi | Chunk di dati dei file (128KB per chunk) |

### Politiche di Cache

- **Write-through**: Le operazioni di scrittura scrivono direttamente sul server e invalidano la cache locale
- **Invalidazione automatica**: Modifiche a file/directory invalidano le cache correlate (file, parent directory, metadati)
- **Eviction LRU**: Quando la cache raggiunge 10MB, i chunk meno recentemente usati vengono rimossi
- **Pulizia periodica**: Entry scadute vengono automaticamente rimosse

### Esempi di comportamento:
```bash
# Prima lettura - cache miss, fetch dal server
$ cat /tmp/remotefs/file.txt

# Seconda lettura entro 30 secondi - cache hit
$ cat /tmp/remotefs/file.txt   # Nessuna richiesta al server

# Dopo modifica - cache invalidata
$ echo "new content" > /tmp/remotefs/file.txt
$ cat /tmp/remotefs/file.txt   # Nuovo fetch dal server
```

## Modalità Daemon

Il client può essere eseguito in background come daemon:

```bash
# Avviare in background
cd clientFS
cargo run --release -- --server http://localhost:8080 \
    --mountpoint /tmp/remotefs \
    --daemon \
    --pidfile /tmp/clientfs.pid \
    --logfile /tmp/clientfs.log

# Verificare che sia in esecuzione
cat /tmp/clientfs.pid

# Vedere i log
tail -f /tmp/clientfs.log

# Fermare il daemon
cargo run --release -- --mountpoint /tmp/remotefs --stop
```

## Recovery Automatico Mount Point

Il client gestisce automaticamente stati zombie del mount point:

- **ENOTCONN (Transport endpoint not connected)**: Smontaggio forzato e riutilizzo
- **Cleanup automatico**: Tentativi con `fusermount -u`, `fusermount -uz`, e `umount -l`
- **Creazione automatica**: Se il mount point non esiste, viene creato automaticamente

## Sviluppo

### Struttura del progetto:
```
.
├── README.md
├── specifiche.md
├── requirements.txt
├── src/server/             # Server Rust (Actix)
│   ├── main.rs
│   └── handlers.rs
└── clientFS/               # Client FUSE Rust
    ├── Cargo.toml
    └── src/
        ├── main.rs         # Entry point del client
        ├── api_client.rs   # Client HTTP per le API
        └── filesystem.rs   # Implementazione FUSE + Cache
```

### Test:
```bash
# Avviare il server in un terminale
cargo run --bin server

# Avviare il client in un altro terminale
cd clientFS
cargo run -- --server http://localhost:8080 --mountpoint /tmp/remotefs --verbose

# Testare in un terzo terminale
cd /tmp/remotefs
ls -la
echo "test" > file.txt
cat file.txt
```

## Troubleshooting

### Il client non si compila:
- Assicurarsi di avere installato `libfuse3-dev` e `build-essential`
- Verificare che Rust sia aggiornato: `rustup update`

### Il client non si monta:
- Verificare che il server sia in esecuzione: `curl http://localhost:8080/health`
- Verificare che FUSE sia disponibile: `fusermount3 --version`
- Verificare i permessi sulla directory di mount
- Provare con `sudo` se necessario

### Errori di permessi:
- Il client può richiedere l'opzione `allow_other` in `/etc/fuse.conf`
- Alcuni sistemi richiedono di essere nel gruppo `fuse`: `sudo usermod -a -G fuse $USER`

## Licenza

Progetto didattico per il corso di Programmazione di Sistema.

