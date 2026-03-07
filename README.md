## Remote File System (Rust)

Un filesystem remoto implementato in Rust che espone un **mount FUSE locale** e salva i dati su un **server REST**.  
Permette di usare file e directory remoti come se fossero locali (ls, cat, echo, mkdir, mv, rm, chmod, ecc.).

---

## 1. Funzionalità principali

- **Filesystem locale FUSE**: mount di una directory (es. `/tmp/remotefs`) che rappresenta il filesystem remoto.
- **Operazioni supportate**:
  - Lettura/scrittura file
  - Creazione ed eliminazione file
  - Creazione ed eliminazione directory
  - Rinomina/spostamento
- **Server REST in Rust/Actix**:
  - API per listare directory, leggere/scrivere file, creare/rimuovere, rinominare, cambiare permessi.
- **Autenticazione JWT**:
  - Il client chiede una password, chiama `/auth` e usa un token JWT in tutte le richieste.
- **Cache locale con TTL**:
  - Metadati, listing directory e chunk file con TTL + eviction LRU (10MB).
- **Politica write-through**:
  - Ogni modifica scrive **subito** sul server e invalida le cache correlate.
- **Modalità daemon + graceful shutdown**:
  - Possibilità di eseguire il client in background.
  - Gestione di SIGINT/SIGTERM, unmount pulito, svuotamento cache e chiusura handle.

---

## 2. Prerequisiti

### 2.1. Toolchain Rust

Serve Rust + Cargo:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update    # opzionale ma consigliato
```

### 2.2. FUSE 3 + build tools (Linux)

Installare FUSE3 e tool di build:

```bash
# Debian / Ubuntu
sudo apt-get install fuse3 libfuse3-dev build-essential

# Fedora
sudo dnf install fuse3 fuse3-devel

# Arch
sudo pacman -S fuse3
```

---

## 3. Struttura del progetto

```text
.
├── README.md
├── specifiche.md
├── requirements.txt
├── start_server.sh         # Script per avviare il server
├── start_client.sh         # Script per avviare il client
├── src/
│   └── server/
│       ├── main.rs         # Entry point server (Actix)
│       └── handlers.rs     # Handler REST
└── clientFS/
    ├── Cargo.toml
    └── src/
        ├── main.rs         # Entry point client
        ├── api_client.rs   # Client HTTP (REST + JWT + mapping errori)
        ├── filesystem.rs   # Implementazione FUSE + write-through
        └── cache.rs        # Gestione cache (TTL + LRU)
```

Il server usa la directory `server_storage` (creata automaticamente nella root del progetto) come **storage locale**.

### 3.1 Architettura

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

--- 

## 4. Guida rapida: step da seguire per utilizzo

### 4.1. Clonare la repository

```bash
git clone < URL_DELLA_REPO >
cd Progetto_Malnati_FileSystem
```

### 4.2. Compilare server e client (opzionale, gli script lo fanno comunque)

- **Server**:

```bash
cargo build --bin server
```

- **Client**:

```bash
cd clientFS
cargo build --release
cd ..
```

---

### 4.3. Avviare il server

Metodo consigliato (script):

```bash
./start_server.sh
```

Equivalente manuale:

```bash
cd /percorso/della/repo
cargo run --bin server
# Server in ascolto su http://0.0.0.0:8080
# Storage in ./server_storage
```

Puoi verificare che sia attivo:

```bash
curl -i http://localhost:8080/health
```

---

### 4.4. Avviare il client FUSE 

### 4.4.1 Modalità foreground

  In un **nuovo terminale**:

  ```bash
  # Crea la directory di mount (se non esiste)
  mkdir -p /tmp/remotefs

  # Avvia il client
  ./start_client.sh /tmp/remotefs http://localhost:8080 foreground
  ```

  Cosa succede:

  - Il client ti chiede la **password**.
  - Chiama `POST /auth` sul server, riceve un token JWT.
  - Monta `/tmp/remotefs` come filesystem FUSE.

  > Per smontare in questa modalità: premi **Ctrl+C** nel terminale del client.

### 4.4.2 Avviare il client in modalità daemon (background) 

  ```bash
  # Avvia in background
  ./start_client.sh /tmp/remotefs http://localhost:8080 daemon

  # Per fermare il daemon
  ./start_client.sh /tmp/remotefs http://localhost:8080 stop
  ```

  In modalità daemon:

  - Il client scrive i log (ad es. in `/tmp/clientfs.log`, a seconda della configurazione nel codice).
  - Fa graceful shutdown su SIGINT/SIGTERM (unmount, svuota cache, chiude handle, rimuove pidfile).

---

### 4.6. Usare il filesystem remoto

In un **terzo terminale**:

```bash
cd /tmp/remotefs

# Esplorare
ls -la

# Creare e leggere file
echo "Hello World" > hello.txt
cat hello.txt

# Creazione Directory
mkdir testdir
mv hello.txt testdir/
ls -la testdir

# Cancellare
rm testdir/hello.txt
rmdir testdir
```

## 5. API del server 

Il server espone le seguenti API RESTful (autenticate con JWT):

- **Autenticazione**
  - `POST /auth` – autentica il client, restituisce un token JWT.

- **Directory & file**
  - `GET /list/<path>` – lista il contenuto di una directory (nome, tipo, size, mtime, ctime, permessi).
  - `GET /files/<path>` – legge il contenuto di un file.
    - Supporta header `Range: bytes=start-end` per letture parziali.
  - `PUT /files/<path>` – scrive (o sovrascrive) l’intero contenuto di un file.
  - `PATCH /files/<path>` – scrittura parziale con header `Content-Range: bytes start-end/*`.
  - `HEAD /files/<path>` – restituisce solo metadati (Content-Length, Last-Modified).

- **Gestione albero file**
  - `POST /mkdir/<path>` – crea una directory (ricorsivamente).
  - `DELETE /files/<path>` – elimina un file o directory.
  - `POST /rename` – rinomina o sposta file/directory.

- **Attributi**
  - `PATCH /attrs/<path>` – aggiorna i permessi (`mode`).

- **Healthcheck**
  - `GET /health` – verifica che il server sia in esecuzione.

---

## 6. Cache locale con TTL (client)

Il client implementa una cache multilivello per ridurre le richieste al server:

- **Metadati (inode)**:
  - TTL: **5 secondi**.
- **Directory listing**:
  - TTL: **3 secondi**.
- **Dati file (chunk)**:
  - TTL: **10 secondi**.
  - Chunk da **128KB**, limite globale **10MB** con eviction **LRU**.

Politiche:

- **Write-through**: ogni operazione di scrittura/rename/chmod scrive subito sul server.
- **Invalidazione automatica**:
  - Dopo una modifica, vengono invalidate:
    - cache del file,
    - cache della directory interessata,
    - metadati associati (inode).
- **Pulizia periodica**:
  - Entry scadute vengono rimosse automaticamente durante le operazioni (es. `getattr` sulla root).

---

## 7. Gestione errori HTTP → POSIX

Il client mappa gli errori HTTP in errno POSIX, così i programmi vedono errori “normali”:

| HTTP    | Errno POSIX     | Significato                       |
|---------|-----------------|-----------------------------------|
| 400     | `EINVAL`        | Parametri non validi              |
| 401/403 | `EACCES`        | Permesso negato                   |
| 404     | `ENOENT`        | File o directory non trovata      |
| 405     | `ENOSYS`        | Operazione non supportata         |
| 409     | `EEXIST`        | Risorsa già esistente             |
| 413     | `EFBIG`         | File troppo grande                |
| 500     | `EIO`           | Errore I/O del server             |
| 503     | `EAGAIN`        | Servizio non disponibile          |
| 507     | `ENOSPC`        | Spazio insufficiente              |
| Timeout | `ETIMEDOUT`     | Timeout connessione               |
| Network | `EHOSTUNREACH`  | Server non raggiungibile          |

---

## 8. Troubleshooting (problemi comuni)

- **Il client non si compila**
  - Verifica di aver installato `libfuse3-dev` e `build-essential`.
  - Aggiorna Rust: `rustup update`.

- **Il client non monta**
  - Controlla che il server sia in esecuzione: `curl http://localhost:8080/health`.
  - Verifica che FUSE3 sia disponibile: `fusermount3 --version`.
  - Verifica i permessi sulla directory di mount.
  - Prova con `sudo` se necessario.

- **Errori di permessi (401/403)**
  - Password errata all’avvio del client.
  - `JWT_SECRET` lato server cambiato senza riavviare i client.

- **Mountpoint “zombie” (ENOTCONN)**
  - Il client cerca di gestire automaticamente questi casi.
  - In caso di problemi, prova:
    - `fusermount3 -u /tmp/remotefs`  
    - oppure `sudo umount -l /tmp/remotefs` e riavvia il client.

---

## 9. Licenza / contesto

Progetto didattico per il corso di **Programmazione di Sistema**, non destinato alla produzione.