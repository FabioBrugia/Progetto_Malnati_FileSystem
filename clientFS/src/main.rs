mod api_client;
mod auth;
mod cache;
mod cli;
mod daemon;
mod error;
mod filesystem;

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::{Arc, Mutex};

use api_client::ApiClient;
use cli::Args;
use filesystem::RemoteFS;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Se richiesto --stop, ferma il daemon
    if args.stop {
        return daemon::stop_daemon(&args.pidfile, &args.mountpoint);
    }

    // Autenticazione
    let password = auth::ask_password();
    let token = auth::authenticate(&args.server, &password).await?;

    // Se daemon mode, daemonizza prima di tutto
    if args.daemon {
        daemon::daemonize(&args.pidfile, &args.logfile, &args.mountpoint)?;
    }

    // Inizializza logging (dopo daemonize, per non perdere l'output)
    cli::init_logging(args.verbose);

    log::info!("Remote File System Client");
    log::info!("Server: {}", args.server);
    log::info!("Mount point: {}", args.mountpoint.display());

    // Verifica e prepara il mount point
    daemon::ensure_mountpoint(&args.mountpoint)?;

    // Crea il client API con l'handle del runtime Tokio
    let runtime_handle = tokio::runtime::Handle::current();
    let api_client = ApiClient::new(args.server.clone(), token, runtime_handle)
        .context("Failed to create API client")?;

    // Test connessione al server
    log::info!("Testing connection to server...");
    api_client
        .health_check()
        .map_err(|e| anyhow::anyhow!("{}", e))
        .context("Failed to connect to server. Is the server running?")?;
    log::info!("Successfully connected to server");

    // Crea e monta il filesystem
    let fs = RemoteFS::new(api_client);

    #[cfg(target_os = "linux")]
    let unmount_hint = format!("fusermount -u {}", args.mountpoint.display());
    #[cfg(target_os = "macos")]
    let unmount_hint = format!("umount {}", args.mountpoint.display());
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let unmount_hint = format!("umount {}", args.mountpoint.display());

    log::info!("Mounting filesystem...");
    log::info!("Use Ctrl+C or '{}' to unmount", unmount_hint);

    // Condividiamo l'unmounter per il graceful shutdown
    let unmounter: Arc<Mutex<Option<fuser::SessionUnmounter>>> = Arc::new(Mutex::new(None));
    let unmounter_for_signal = unmounter.clone();

    // Registra il signal handler per SIGINT (Ctrl+C) e SIGTERM
    let mount_display = args.mountpoint.display().to_string();
    let mountpoint_for_signal = args.mountpoint.clone();
    let pidfile_for_signal = args.pidfile.clone();
    let is_daemon = args.daemon;
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to register SIGTERM handler");

            tokio::select! {
                _ = ctrl_c => {
                    log::info!("Ricevuto SIGINT (Ctrl+C). Smontaggio in corso...");
                }
                _ = sigterm.recv() => {
                    log::info!("Ricevuto SIGTERM. Smontaggio in corso...");
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
            log::info!("Ricevuto segnale di terminazione (Ctrl+C). Smontaggio in corso...");
        }

        // Esegui l'unmount
        if let Ok(mut guard) = unmounter_for_signal.lock() {
            if let Some(ref mut u) = *guard {
                if daemon::is_fuse_mounted(&mountpoint_for_signal) {
                    log::info!("Smontaggio filesystem da {}...", mount_display);
                    if let Err(e) = u.unmount() {
                        log::error!("Errore durante lo smontaggio: {}", e);
                    }
                } else {
                    log::info!(
                        "Mountpoint {} non risulta montato, salto lo smontaggio.",
                        mount_display
                    );
                }
            }
        }

        // Rimuovi il PID file se in daemon mode
        if is_daemon {
            let _ = std::fs::remove_file(&pidfile_for_signal);
            log::info!("PID file rimosso.");
        }
    });

    // fuser::Session::run() blocca il thread — lo eseguiamo in spawn_blocking
    let unmounter_for_mount = unmounter.clone();
    let mountpoint_for_mount = args.mountpoint.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        fs.mount(&mountpoint_for_mount, unmounter_for_mount)
            .context("Failed to mount filesystem")
    })
    .await
    .context("Mount task panicked")??;

    log::info!("Filesystem smontato correttamente.");

    // Pulizia PID file dopo lo smontaggio
    if args.daemon && args.pidfile.exists() {
        let _ = std::fs::remove_file(&args.pidfile);
        log::info!("PID file rimosso.");
    }

    Ok(())
}
