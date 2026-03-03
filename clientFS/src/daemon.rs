use anyhow::{Context, Result};
use daemonize::Daemonize;
use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Controlla se un path è attualmente un mountpoint FUSE attivo.
///
/// Legge /proc/mounts e cerca una riga il cui mountpoint corrisponda al path dato.
pub fn is_fuse_mounted(mountpoint: &Path) -> bool {
    let canonical = match fs::canonicalize(mountpoint) {
        Ok(p) => p,
        Err(_) => mountpoint.to_path_buf(),
    };
    let mount_str = canonical.to_string_lossy().to_string();

    if let Ok(file) = fs::File::open("/proc/mounts") {
        let reader = std::io::BufReader::new(file);
        for line in reader.lines().flatten() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[1] == mount_str {
                // Verifica che sia un mount FUSE
                if parts[2].starts_with("fuse") || parts[0].contains("fuse") {
                    return true;
                }
            }
        }
    }
    false
}

/// Smonta un mountpoint FUSE, con retry e fallback lazy-unmount.
///
/// Restituisce Ok(true) se lo smontaggio è riuscito, Ok(false) se non era montato.
pub fn unmount_fuse(mountpoint: &Path) -> Result<bool> {
    if !is_fuse_mounted(mountpoint) {
        return Ok(false);
    }

    // Primo tentativo: fusermount -u
    let output = Command::new("fusermount")
        .arg("-u")
        .arg(mountpoint)
        .output();

    if let Ok(out) = &output {
        if out.status.success() {
            // Attendi un attimo e verifica
            thread::sleep(Duration::from_millis(200));
            if !is_fuse_mounted(mountpoint) {
                return Ok(true);
            }
        }
    }

    // Secondo tentativo: lazy unmount
    let _ = Command::new("fusermount")
        .arg("-uz")
        .arg(mountpoint)
        .output();

    thread::sleep(Duration::from_millis(500));

    if !is_fuse_mounted(mountpoint) {
        return Ok(true);
    }

    // Terzo tentativo: sudo umount -l
    let _ = Command::new("sudo")
        .arg("umount")
        .arg("-l")
        .arg(mountpoint)
        .output();

    thread::sleep(Duration::from_millis(500));

    Ok(!is_fuse_mounted(mountpoint))
}

/// Ferma il daemon in esecuzione.
///
/// Smonta il filesystem FUSE e termina il processo daemon.
pub fn stop_daemon(pidfile: &Path, mountpoint: &Path) -> Result<()> {
    // Prima smonta il filesystem
    println!("Smontaggio filesystem da {}...", mountpoint.display());

    match unmount_fuse(mountpoint) {
        Ok(true) => println!("Filesystem smontato correttamente."),
        Ok(false) => println!("Il filesystem non era montato."),
        Err(e) => println!("Avviso durante lo smontaggio: {}", e),
    }

    // Leggi il PID e termina il processo
    if pidfile.exists() {
        let pid_str = fs::read_to_string(pidfile)
            .context("Impossibile leggere il PID file")?;
        let pid: i32 = pid_str.trim().parse()
            .context("PID non valido")?;

        // Controlla se il processo è ancora in esecuzione
        let process_alive = unsafe { libc::kill(pid, 0) } == 0;

        if process_alive {
            println!("Terminazione processo daemon (PID: {})...", pid);

            // Invia SIGTERM
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }

            // Attendi fino a 5 secondi che il processo termini
            for _ in 0..50 {
                thread::sleep(Duration::from_millis(100));
                let still_alive = unsafe { libc::kill(pid, 0) } == 0;
                if !still_alive {
                    break;
                }
            }

            // Se ancora vivo, invia SIGKILL
            let still_alive = unsafe { libc::kill(pid, 0) } == 0;
            if still_alive {
                println!("Il processo non risponde, invio SIGKILL...");
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        } else {
            println!("Il processo daemon (PID: {}) non è più in esecuzione.", pid);
        }

        // Rimuovi il PID file
        let _ = fs::remove_file(pidfile);
        println!("Daemon fermato.");
    } else {
        println!("Nessun daemon in esecuzione (PID file non trovato).");
        // Prova comunque a smontare nel caso il pidfile sia stato perso
        if is_fuse_mounted(mountpoint) {
            println!("Tuttavia il mountpoint risulta ancora montato. Smontaggio forzato...");
            let _ = unmount_fuse(mountpoint);
        }
    }

    Ok(())
}

/// Avvia il processo in modalità daemon (background).
///
/// Dopo questa chiamata, il processo è stato forkato e il padre è terminato.
/// Il codice che segue viene eseguito nel processo figlio (daemon).
pub fn daemonize(pidfile: &Path, logfile: &Path, mountpoint: &Path) -> Result<()> {
    println!("Avvio in modalità daemon...");
    println!("PID file: {}", pidfile.display());
    println!("Log file: {}", logfile.display());
    println!("Mount point: {}", mountpoint.display());

    let stdout = File::create(logfile)
        .context("Impossibile creare il file di log")?;
    let stderr = stdout.try_clone()
        .context("Impossibile duplicare il file di log")?;

    let daemonize = Daemonize::new()
        .pid_file(pidfile)
        .chown_pid_file(true)
        .working_directory("/tmp")
        .stdout(stdout)
        .stderr(stderr);

    daemonize.start()
        .map_err(|e| anyhow::anyhow!("Impossibile avviare il daemon: {}", e))?;

    Ok(())
}

/// Verifica e prepara il mount point.
///
/// Se il mount point è già montato con FUSE, lo smonta prima.
/// Se è in stato ENOTCONN (FUSE zombie), tenta lo smontaggio forzato.
/// Crea la directory se non esiste.
pub fn ensure_mountpoint(dir: &PathBuf) -> Result<()> {
    log::debug!("Verifica mountpoint: {}", dir.display());

    // Controlla se è già montato con FUSE → smonta e ricollega
    if is_fuse_mounted(dir) {
        log::warn!(
            "Il mountpoint {} è già montato con FUSE. Smontaggio per ricollegamento...",
            dir.display()
        );
        match unmount_fuse(dir) {
            Ok(true) => log::info!("Vecchio mount FUSE smontato con successo."),
            Ok(false) => log::warn!("Non è stato possibile smontare il vecchio mount."),
            Err(e) => log::warn!("Errore durante lo smontaggio del vecchio mount: {}", e),
        }
        // Attendi un attimo per assicurarsi che il kernel rilasci la risorsa
        thread::sleep(Duration::from_millis(300));
    }

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
                        log::warn!(
                            "Mountpoint in stato 'Transport endpoint not connected' (ENOTCONN). \
                             Provo smontaggio forzato..."
                        );
                        let _ = Command::new("fusermount").arg("-u").arg(dir).output();
                        let _ = Command::new("fusermount").arg("-uz").arg(dir).output();
                        let _ = Command::new("sudo").arg("umount").arg("-l").arg(dir).output();
                        // Attendi e riprova
                        thread::sleep(Duration::from_millis(500));
                        // Se ancora in stato zombie, prova a rimuovere
                        match fs::metadata(dir) {
                            Ok(_) => {}
                            Err(_) => {
                                let _ = fs::remove_dir(dir);
                            }
                        }
                    } else {
                        log::warn!(
                            "Errore nel metadata del mountpoint (code={}): {}",
                            code, e
                        );
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
                    anyhow::bail!(
                        "Mountpoint ancora in stato ENOTCONN. Chiudi ogni shell o processo \
                         che usa {} e riprova oppure usa un mount point diverso \
                         (es. /tmp/remotefs2)",
                        dir.display()
                    );
                }
            }
            Err(e).context("Failed to create mount point directory")
        }
    }
}

