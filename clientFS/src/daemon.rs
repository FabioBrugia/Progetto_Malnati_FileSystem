use anyhow::{Context, Result};
use daemonize::Daemonize;
use std::fs;
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Checks if a path is currently an active FUSE mountpoint.
pub fn is_fuse_mounted(mountpoint: &Path) -> bool {
    let canonical = match fs::canonicalize(mountpoint) {
        Ok(p) => p,
        Err(_) => mountpoint.to_path_buf(),
    };
    let mount_str = canonical.to_string_lossy().to_string();

    #[cfg(target_os = "linux")]
    {
        if let Ok(file) = fs::File::open("/proc/mounts") {
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().flatten() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[1] == mount_str {
                    // Verify it's a FUSE mount
                    if parts[2].starts_with("fuse") || parts[0].contains("fuse") {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("mount").output() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let needle = format!(" on {} (", mount_str);
            for line in stdout.lines() {
                if line.contains(&needle) && line.to_lowercase().contains("fuse") {
                    return true;
                }
            }
        }
        return false;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

/// Unmounts a FUSE mountpoint, with retry and lazy-unmount fallback.
///
/// Returns Ok(true) if unmount succeeded, Ok(false) if it was not mounted.
pub fn unmount_fuse(mountpoint: &Path) -> Result<bool> {
    if !is_fuse_mounted(mountpoint) {
        return Ok(false);
    }

    #[cfg(target_os = "linux")]
    {
        // First attempt: fusermount -u
        let output = Command::new("fusermount")
            .arg("-u")
            .arg(mountpoint)
            .output();

        if let Ok(out) = &output {
            if out.status.success() {
                // Wait briefly and verify
                thread::sleep(Duration::from_millis(200));
                if !is_fuse_mounted(mountpoint) {
                    return Ok(true);
                }
            }
        }

        // Second attempt: lazy unmount
        let _ = Command::new("fusermount")
            .arg("-uz")
            .arg(mountpoint)
            .output();

        thread::sleep(Duration::from_millis(500));

        if !is_fuse_mounted(mountpoint) {
            return Ok(true);
        }

        // Third attempt: sudo umount -l
        let _ = Command::new("sudo")
            .arg("umount")
            .arg("-l")
            .arg(mountpoint)
            .output();

        thread::sleep(Duration::from_millis(500));
        return Ok(!is_fuse_mounted(mountpoint));
    }

    #[cfg(target_os = "macos")]
    {
        // First attempt: umount
        let output = Command::new("umount").arg(mountpoint).output();

        if let Ok(out) = &output {
            if out.status.success() {
                thread::sleep(Duration::from_millis(200));
                if !is_fuse_mounted(mountpoint) {
                    return Ok(true);
                }
            }
        }

        // Second attempt: force unmount
        let _ = Command::new("umount").arg("-f").arg(mountpoint).output();

        thread::sleep(Duration::from_millis(500));
        return Ok(!is_fuse_mounted(mountpoint));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Fallback: try standard umount
        let output = Command::new("umount").arg(mountpoint).output();
        if let Ok(out) = &output {
            if out.status.success() {
                thread::sleep(Duration::from_millis(200));
                return Ok(!is_fuse_mounted(mountpoint));
            }
        }
        Ok(false)
    }
}

/// Stops the running daemon.
///
/// Unmounts the FUSE filesystem and terminates the daemon process.
pub fn stop_daemon(pidfile: &Path, mountpoint: &Path) -> Result<()> {
    // First unmount the filesystem
    println!("Unmounting filesystem from {}...", mountpoint.display());

    match unmount_fuse(mountpoint) {
        Ok(true) => println!("Filesystem unmounted successfully."),
        Ok(false) => println!("Filesystem was not mounted."),
        Err(e) => println!("Warning during unmount: {}", e),
    }

    // Read the PID and terminate the process
    if pidfile.exists() {
        let pid_str = fs::read_to_string(pidfile)
            .context("Unable to read PID file")?;
        let pid: i32 = pid_str.trim().parse()
            .context("Invalid PID")?;

        // Check if the process is still running
        let process_alive = unsafe { libc::kill(pid, 0) } == 0;

        if process_alive {
            println!("Terminating daemon process (PID: {})...", pid);

            // Send SIGTERM
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }

            // Wait up to 5 seconds for the process to terminate
            for _ in 0..50 {
                thread::sleep(Duration::from_millis(100));
                let still_alive = unsafe { libc::kill(pid, 0) } == 0;
                if !still_alive {
                    break;
                }
            }

            // If still alive, send SIGKILL
            let still_alive = unsafe { libc::kill(pid, 0) } == 0;
            if still_alive {
                println!("Process not responding, sending SIGKILL...");
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        } else {
            println!("Daemon process (PID: {}) is no longer running.", pid);
        }

        // Remove the PID file
        let _ = fs::remove_file(pidfile);
        println!("Daemon stopped.");
    } else {
        println!("No daemon running (PID file not found).");
        // Try to unmount anyway in case the pidfile was lost
        if is_fuse_mounted(mountpoint) {
            println!("However the mountpoint is still mounted. Forcing unmount...");
            let _ = unmount_fuse(mountpoint);
        }
    }

    Ok(())
}

/// Starts the process in daemon (background) mode.
///
/// After this call, the process has been forked and the parent has exited.
/// The code that follows runs in the child process (daemon).
pub fn daemonize(pidfile: &Path, logfile: &Path, mountpoint: &Path) -> Result<()> {
    println!("Starting in daemon mode...");
    println!("PID file: {}", pidfile.display());
    println!("Log file: {}", logfile.display());
    println!("Mount point: {}", mountpoint.display());

    let stdout = File::create(logfile)
        .context("Unable to create log file")?;
    let stderr = stdout.try_clone()
        .context("Unable to duplicate log file")?;

    let daemonize = Daemonize::new()
        .pid_file(pidfile)
        .chown_pid_file(true)
        .working_directory("/tmp")
        .stdout(stdout)
        .stderr(stderr);

    daemonize.start()
        .map_err(|e| anyhow::anyhow!("Unable to start daemon: {}", e))?;

    Ok(())
}

/// Verifies and prepares the mount point.
///
/// If the mount point is already mounted with FUSE, it unmounts it first.
/// If it is in ENOTCONN state (FUSE zombie), it attempts a forced unmount.
/// Creates the directory if it does not exist.
pub fn ensure_mountpoint(dir: &PathBuf) -> Result<()> {
    log::debug!("Checking mountpoint: {}", dir.display());

    // Check if already mounted with FUSE → unmount and reconnect
    if is_fuse_mounted(dir) {
        log::warn!(
            "Mountpoint {} is already mounted. Unmounting...",
            dir.display()
        );
        match unmount_fuse(dir) {
            Ok(true) => log::info!("Old mountpoint unmounted correctly."),
            Ok(false) => log::warn!("Mountpoint was not mounted, but is_fuse_mounted returned true. Check manually: {}", dir.display()),
            Err(e) => log::warn!("Error during unmount operation: {}", e),
        }
        // Wait briefly to ensure the kernel releases the resource
        thread::sleep(Duration::from_millis(300));
    }

    if dir.exists() {
        // Check if the path is in ENOTCONN state (FUSE zombie endpoint)
        match fs::metadata(dir) {
            Ok(meta) => {
                if !meta.is_dir() {
                    anyhow::bail!("Mount point exist but is not a directory: {}", dir.display());
                }
            }
            Err(e) => {
                if let Some(code) = e.raw_os_error() {
                    if code == libc::ENOTCONN {
                        log::warn!(
                            "Mountpoint state: 'Transport endpoint not connected' (ENOTCONN). \
                             Attempting forced unmount..."
                        );

                        #[cfg(target_os = "linux")]
                        {
                            let _ = Command::new("fusermount").arg("-u").arg(dir).output();
                            let _ = Command::new("fusermount").arg("-uz").arg(dir).output();
                            let _ = Command::new("sudo").arg("umount").arg("-l").arg(dir).output();
                        }

                        #[cfg(target_os = "macos")]
                        {
                            let _ = Command::new("umount").arg(dir).output();
                            let _ = Command::new("umount").arg("-f").arg(dir).output();
                        }

                        // Wait and retry
                        thread::sleep(Duration::from_millis(500));
                        // If still in zombie state, try to remove
                        match fs::metadata(dir) {
                            Ok(_) => {}
                            Err(_) => {
                                let _ = fs::remove_dir(dir);
                            }
                        }
                    } else {
                        log::warn!(
                            "Error metadata of mountpoint (code={}): {}",
                            code, e
                        );
                    }
                }
            }
        }
    }

    // Create if it doesn't exist or after cleanup
    match fs::create_dir_all(dir) {
        Ok(_) => {
            log::debug!("Mountpoint ready: {}", dir.display());
            Ok(())
        }
        Err(e) => {
            if let Some(code) = e.raw_os_error() {
                if code == libc::EEXIST {
                    log::debug!("Directory already exists: {}", dir.display());
                    return Ok(());
                } else if code == libc::ENOTCONN {
                    anyhow::bail!(
                        "Mountpoint still in ENOTCONN state. Close all shells or processes \
                         using {} and retry, or use another mount point \
                         (e.g. /tmp/remotefs2)",
                        dir.display()
                    );
                }
            }
            Err(e).context("Failed to create mount point directory")
        }
    }
}

