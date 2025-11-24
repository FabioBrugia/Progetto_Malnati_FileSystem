# Remote File System (Rust)

Un filesystem remoto implementato in Rust che presenta un mount point locale, rispecchiando la struttura di un file system ospitato su un server remoto.

## Caratteristiche

- ✅ Interfaccia filesystem locale che interagisce con storage remoto
- ✅ Operazioni standard sui file (lettura, scrittura, creazione, eliminazione, rinomina)
- ✅ Supporto completo per Linux usando FUSE
- ✅ Server RESTful implementato in Rust/Actix Web
- ✅ Client FUSE implementato in Rust

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

Il server parte su `http://0.0.0.0:9000` e utilizza `/tmp/remote_fs_test` come storage locale (creato automaticamente).

### 2. Creare un mount point e avviare il client:
```bash
# Creare la directory di mount
mkdir -p /tmp/remotefs

# Avviare il client (in un altro terminale)
cd clientFS
cargo run --release -- --server http://localhost:8080 --mountpoint /tmp/remotefs --verbose
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

## Sviluppo

### Struttura del progetto:
```
.
├── README.md
├── specifiche.md
├── requirements.txt
├── src/server/             # Server Rust (Actix)
├── clientFS/
└── clientFS/
    ├── Cargo.toml
    └── src/
        ├── main.rs         # Entry point del client
        ├── api_client.rs   # Client HTTP per le API
        └── filesystem.rs   # Implementazione FUSE
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
- Verificare che il server sia in esecuzione: `curl http://localhost:9000/health`
- Verificare che FUSE sia disponibile: `fusermount3 --version`
- Verificare i permessi sulla directory di mount
- Provare con `sudo` se necessario

### Errori di permessi:
- Il client può richiedere l'opzione `allow_other` in `/etc/fuse.conf`
- Alcuni sistemi richiedono di essere nel gruppo `fuse`: `sudo usermod -a -G fuse $USER`

## Licenza

Progetto didattico per il corso di Programmazione di Sistema.

