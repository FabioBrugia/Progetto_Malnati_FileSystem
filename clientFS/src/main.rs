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

/// Entry point: we do NOT use #[tokio::main] because daemonize (fork)
/// must happen BEFORE any Tokio runtime is created. A forked multi-thread
/// runtime is corrupted and network I/O silently fails.
fn main() -> Result<()> {
    let args = Args::parse();

    // If --stop was requested, stop the daemon (no runtime needed)
    if args.stop {
        return daemon::stop_daemon(&args.pidfile, &args.mountpoint);
    }

    // Authentication: we need a temporary runtime just for the login request.
    // This runtime is dropped before fork() so the daemon child starts clean.
    let password = auth::ask_password();
    let token = {
        let tmp_rt = tokio::runtime::Runtime::new()
            .context("Failed to create temporary runtime for authentication")?;
        tmp_rt.block_on(auth::authenticate(&args.server, &password))?
    };
    // tmp_rt is dropped here — safe to fork

    // If daemon mode, fork BEFORE creating the real Tokio runtime
    if args.daemon {
        daemon::daemonize(&args.pidfile, &args.logfile, &args.mountpoint)?;
    }

    // Now create the real Tokio runtime (in the daemon child if forked)
    let runtime = tokio::runtime::Runtime::new()
        .context("Failed to create Tokio runtime")?;

    runtime.block_on(async_main(args, token))
}

/// Actual async logic, executed inside a fresh Tokio runtime.
async fn async_main(args: Args, token: String) -> Result<()> {
    // Initialize logging (after daemonize, so output goes to logfile)
    cli::init_logging(args.verbose);

    log::info!("Remote File System Client");
    log::info!("Server: {}", args.server);
    log::info!("Mount point: {}", args.mountpoint.display());

    // Verify and prepare the mount point
    daemon::ensure_mountpoint(&args.mountpoint)?;

    // Create the API client with the current (fresh) runtime handle
    let runtime_handle = tokio::runtime::Handle::current();
    let api_client = ApiClient::new(args.server.clone(), token, runtime_handle)
        .context("Failed to create API client")?;

    // Test connection to server
    log::info!("Testing connection to server...");
    api_client
        .health_check()
        .map_err(|e| anyhow::anyhow!("{}", e))
        .context("Failed to connect to server. Is the server running?")?;
    log::info!("Successfully connected to server");

    // Create and mount the filesystem
    let fs = RemoteFS::new(api_client);

    #[cfg(target_os = "linux")]
    let unmount_hint = format!("fusermount -u {}", args.mountpoint.display());
    #[cfg(target_os = "macos")]
    let unmount_hint = format!("umount {}", args.mountpoint.display());
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let unmount_hint = format!("umount {}", args.mountpoint.display());

    log::info!("Mounting filesystem...");
    log::info!("Use Ctrl+C or '{}' to unmount", unmount_hint);

    // Shared unmounter for graceful shutdown
    let unmounter: Arc<Mutex<Option<fuser::SessionUnmounter>>> = Arc::new(Mutex::new(None));
    let unmounter_for_signal = unmounter.clone();

    // Register signal handler for SIGINT (Ctrl+C) and SIGTERM
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
                    log::info!("Received SIGINT (Ctrl+C). Unmounting...");
                }
                _ = sigterm.recv() => {
                    log::info!("Received SIGTERM. Unmounting...");
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
            log::info!("Received termination signal (Ctrl+C). Unmounting...");
        }

        // Perform unmount
        if let Ok(mut guard) = unmounter_for_signal.lock() {
            if let Some(ref mut u) = *guard {
                if daemon::is_fuse_mounted(&mountpoint_for_signal) {
                    log::info!("Unmounting filesystem from {}...", mount_display);
                    if let Err(e) = u.unmount() {
                        log::error!("Error during unmount: {}", e);
                    }
                } else {
                    log::info!(
                        "Mountpoint {} is not mounted, skipping unmount.",
                        mount_display
                    );
                }
            }
        }

        // Remove the PID file if in daemon mode
        if is_daemon {
            let _ = std::fs::remove_file(&pidfile_for_signal);
            log::info!("PID file removed.");
        }
    });

    // fuser::Session::run() blocks the thread — run it in spawn_blocking
    let unmounter_for_mount = unmounter.clone();
    let mountpoint_for_mount = args.mountpoint.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        fs.mount(&mountpoint_for_mount, unmounter_for_mount)
            .context("Failed to mount filesystem")
    })
    .await
    .context("Mount task panicked")??;

    log::info!("Filesystem unmounted successfully.");

    // Cleanup PID file after unmount
    if args.daemon && args.pidfile.exists() {
        let _ = std::fs::remove_file(&args.pidfile);
        log::info!("PID file removed.");
    }

    Ok(())
}
