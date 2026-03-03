/// Errore personalizzato che mappa gli errori HTTP ai codici POSIX (errno).
///
/// Utilizzato per tradurre le risposte HTTP del server in errori
/// comprensibili dal layer FUSE.
#[derive(Debug)]
pub struct ApiError {
    /// Codice errno POSIX (es. ENOENT, EIO, EACCES)
    pub errno: i32,
    /// Messaggio descrittivo dell'errore
    pub message: String,
}

impl ApiError {
    /// Mappa un codice di stato HTTP al corrispondente errno POSIX.
    ///
    /// # Mappature principali
    /// - 400 → EINVAL (parametri non validi)
    /// - 401/403 → EACCES (permesso negato)
    /// - 404 → ENOENT (file non trovato)
    /// - 409 → EEXIST (risorsa già esistente)
    /// - 500 → EIO (errore I/O generico)
    /// - 507 → ENOSPC (spazio insufficiente)
    pub fn from_status(status: reqwest::StatusCode, operation: &str) -> Self {
        let (errno, message) = match status.as_u16() {
            // Errori client
            400 => (libc::EINVAL, format!("{}: Bad request", operation)),
            401 | 403 => (libc::EACCES, format!("{}: Permission denied", operation)),
            404 => (libc::ENOENT, format!("{}: File or directory not found", operation)),
            405 => (libc::ENOSYS, format!("{}: Operation not supported", operation)),
            408 => (libc::ETIMEDOUT, format!("{}: Request timeout", operation)),
            409 => (libc::EEXIST, format!("{}: Resource already exists", operation)),
            413 => (libc::EFBIG, format!("{}: File too large", operation)),
            415 => (libc::EINVAL, format!("{}: Unsupported media type", operation)),
            429 => (libc::EAGAIN, format!("{}: Too many requests", operation)),

            // Errori server
            500 => (libc::EIO, format!("{}: Internal server error", operation)),
            501 => (libc::ENOSYS, format!("{}: Not implemented", operation)),
            502 | 503 => (libc::EAGAIN, format!("{}: Service unavailable", operation)),
            504 => (libc::ETIMEDOUT, format!("{}: Gateway timeout", operation)),
            507 => (libc::ENOSPC, format!("{}: Insufficient storage", operation)),

            // Default
            _ => (libc::EIO, format!("{}: HTTP error {}", operation, status.as_u16())),
        };

        Self { errno, message }
    }

    /// Crea un errore a partire da un problema di rete/connessione.
    pub fn from_network_error(operation: &str, err: &reqwest::Error) -> Self {
        let (errno, message) = if err.is_timeout() {
            (libc::ETIMEDOUT, format!("{}: Connection timeout", operation))
        } else if err.is_connect() {
            (libc::EHOSTUNREACH, format!("{}: Cannot connect to server", operation))
        } else {
            (libc::EIO, format!("{}: Network error: {}", operation, err))
        };

        Self { errno, message }
    }

    /// Crea un errore generico di I/O.
    pub fn io_error(operation: &str, detail: &str) -> Self {
        Self {
            errno: libc::EIO,
            message: format!("{}: {}", operation, detail),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (errno: {})", self.message, self.errno)
    }
}

impl std::error::Error for ApiError {}

