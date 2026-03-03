use clap::Parser;
use std::path::PathBuf;

/// Argomenti della riga di comando per il client filesystem remoto.
#[derive(Parser, Debug)]
#[command(
    name = "clientFS",
    author,
    version,
    about = "Client FUSE per filesystem remoto",
    long_about = "Monta un filesystem remoto tramite un server HTTP usando FUSE.\n\
                  Supporta lettura, scrittura, creazione e cancellazione di file e directory."
)]
pub struct Args {
    /// URL del server (es. http://localhost:8080)
    #[arg(short, long, default_value = "http://localhost:8080")]
    pub server: String,

    /// Directory di mount point
    #[arg(short, long)]
    pub mountpoint: PathBuf,

    /// Abilita logging dettagliato (debug)
    #[arg(short, long)]
    pub verbose: bool,

    /// Esegui in background (modalità daemon)
    #[arg(short, long)]
    pub daemon: bool,

    /// Ferma il daemon in esecuzione
    #[arg(long)]
    pub stop: bool,

    /// Percorso del PID file (usato solo con --daemon)
    #[arg(long, default_value = "/tmp/clientfs.pid")]
    pub pidfile: PathBuf,

    /// Percorso del file di log (usato solo con --daemon)
    #[arg(long, default_value = "/tmp/clientfs.log")]
    pub logfile: PathBuf,
}

/// Inizializza il logger con il livello appropriato.
pub fn init_logging(verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(log_level),
    )
    .init();
}

