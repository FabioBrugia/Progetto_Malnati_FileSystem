mod api_client;
mod filesystem;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::Command;
use std::fs;
use daemonize::Daemonize;
use std::fs::File;
use rpassword::read_password;
use std::io::{self, Write};

use api_client::ApiClient;
use filesystem::RemoteFS;
use reqwest::blocking::Client;

#[derive(serde::Deserialize)]
struct AuthResponse {
    token: String,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server URL (e.g., http://localhost:8080)
    #[arg(short, long, default_value = "http://localhost:8080")]
    server: String,

    /// Mount point directory
    #[arg(short, long)]
    mountpoint: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Run in background (daemon mode)
    #[arg(short, long)]
    daemon: bool,

    /// Stop the running daemon
    #[arg(long)]
    stop: bool,

    /// PID file location (only used with --daemon)
    #[arg(long, default_value = "/tmp/clientfs.pid")]
    pidfile: PathBuf,

    /// Log file location (only used with --daemon)
    #[arg(long, default_value = "/tmp/clientfs.log")]
    logfile: PathBuf,
}

fn stop_daemon(pidfile: &PathBuf, mountpoint: &PathBuf) -> Result<()> {
    // Prima smonta il filesystem
    println!("Smontaggio filesystem da {}...", mountpoint.display());
    let _ = Command::new("fusermount").arg("-u").arg(mountpoint).output();
    let _ = Command::new("fusermount").arg("-uz").arg(mountpoint).output();

    // Leggi il PID e termina il processo
    if pidfile.exists() {
        let pid_str = fs::read_to_string(pidfile)
            .context("Impossibile leggere il PID file")?;
        let pid: i32 = pid_str.trim().parse()
            .context("PID non valido")?;

        println!("Terminazione processo daemon (PID: {})...", pid);

        // Invia SIGTERM
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }

        // Rimuovi il PID file
        let _ = fs::remove_file(pidfile);
        println!("Daemon fermato.");
    } else {
        println!("Nessun daemon in esecuzione (PID file non trovato).");
    }

    Ok(())
}

fn ensure_mountpoint(dir: &PathBuf) -> Result<()> {
    log::debug!("Verifica mountpoint: {}", dir.display());
    if dir.exists() {
        // Check se il path è in stato ENOTCONN (endpoint FUSE zombie)
        match fs::metadata(dir) {
            Ok(meta) => {
                if !meta.is_dir() {
                    anyhow::bail!("Mount point esiste ma non è una directory");
                }
            }
            Err(e) => {
                if let Some(code) = e.raw_os_error() {
                    if code == libc::ENOTCONN {
                        log::warn!("Mountpoint in stato 'Transport endpoint not connected' (ENOTCONN). Provo smontaggio forzato...");
                        // Tentativi di smontaggio
                        let _ = Command::new("fusermount").arg("-u").arg(dir).output();
                        let _ = Command::new("fusermount").arg("-uz").arg(dir).output();
                        let _ = Command::new("sudo").arg("umount").arg("-l").arg(dir).output();
                        // Rimozione directory se ancora rotta
                        let _ = fs::remove_dir(dir);
                    } else {
                        log::warn!("Errore nel metadata del mountpoint (code={}): {}", code, e);
                    }
                }
            }
        }
    }

    // Crea se non esiste o dopo pulizia
    match fs::create_dir_all(dir) {
        Ok(_) => {
            log::debug!("Mountpoint pronto: {}", dir.display());
            Ok(())
        }
        Err(e) => {
            if let Some(code) = e.raw_os_error() {
                if code == libc::EEXIST {
                    log::debug!("Directory già esistente: {}", dir.display());
                    return Ok(());
                } else if code == libc::ENOTCONN {
                    anyhow::bail!("Mountpoint ancora in stato ENOTCONN. Chiudi ogni shell o processo che usa {} e riprova oppure usa un mount point diverso (es. /tmp/remotefs2)", dir.display());
                }
            }
            Err(e).context("Failed to create mount point directory")
        }
    }
}

fn ask_password() -> String {
    print!("Password: ");
    io::stdout().flush().unwrap();
    read_password().unwrap()
}

fn main() -> Result<()> {
    let password = ask_password();

    let http_client = Client::new();

    let response = http_client
        .post("http://127.0.0.1:8080/auth")
        .json(&serde_json::json!({
            "password": password
        }))
        .send()
        .expect("Server non raggiungibile");

    if !response.status().is_success() {
        eprintln!("Autenticazione fallita.");
        std::process::exit(1);
    }

    let auth: AuthResponse = response.json().unwrap();
    let token = auth.token;
    let args = Args::parse();

    // Se richiesto --stop, ferma il daemon
    if args.stop {
        return stop_daemon(&args.pidfile, &args.mountpoint);
    }

    // Se daemon mode, daemonizza prima di tutto
    if args.daemon {
        println!("Avvio in modalità daemon...");
        println!("PID file: {}", args.pidfile.display());
        println!("Log file: {}", args.logfile.display());
        println!("Mount point: {}", args.mountpoint.display());

        let stdout = File::create(&args.logfile)
            .context("Impossibile creare il file di log")?;
        let stderr = stdout.try_clone()
            .context("Impossibile duplicare il file di log")?;

        let daemonize = Daemonize::new()
            .pid_file(&args.pidfile)
            .chown_pid_file(true)
            .working_directory("/tmp")
            .stdout(stdout)
            .stderr(stderr);

        match daemonize.start() {
            Ok(_) => {
                // Siamo nel processo daemon - continua sotto
            }
            Err(e) => {
                anyhow::bail!("Impossibile avviare il daemon: {}", e);
            }
        }
    }

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .init();

    log::info!("Remote File System Client");
    log::info!("Server: {}", args.server);
    log::info!("Mount point: {}", args.mountpoint.display());

    // Ensure mountpoint exists and is ready
    ensure_mountpoint(&args.mountpoint)?;

    // Create API client
    let api_client = ApiClient::new(args.server.clone(), token)
        .context("Failed to create API client")?;

    // Test connection to server
    log::info!("Testing connection to server...");
    api_client
        .health_check()
        .map_err(|e| anyhow::anyhow!("{}", e))
        .context("Failed to connect to server. Is the server running?")?;
    log::info!("Successfully connected to server");

    // Create and mount filesystem
    let fs = RemoteFS::new(api_client);

    log::info!("Mounting filesystem...");
    log::info!("Use 'fusermount -u {}' to unmount", args.mountpoint.display());

    let mount_str = args.mountpoint.to_str().unwrap().to_string();

    fs.mount(&mount_str)
        .context("Failed to mount filesystem")?;

    // Dopo ritorno da mount (smontato)
    log::info!("Filesystem smontato.");

    Ok(())
}
