use anyhow::Result;
use reqwest::Client;
use rpassword::read_password;
use std::io::{self, Write};

/// Risposta dal server di autenticazione.
#[derive(serde::Deserialize)]
struct AuthResponse {
    token: String,
}

/// Chiede la password all'utente in modo sicuro (senza echo).
pub fn ask_password() -> String {
    print!("Password: ");
    io::stdout().flush().unwrap();
    read_password().unwrap()
}

/// Esegue l'autenticazione con il server e restituisce il token JWT.
///
/// # Argomenti
/// * `server_url` - URL base del server (es. "http://localhost:8080")
/// * `password` - Password dell'utente
///
/// # Errori
/// Ritorna errore se il server non è raggiungibile o la password è errata.
pub async fn authenticate(server_url: &str, password: &str) -> Result<String> {
    let http_client = Client::new();

    let response = http_client
        .post(format!("{}/auth", server_url))
        .json(&serde_json::json!({
            "password": password
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Server non raggiungibile: {}", e))?;

    if !response.status().is_success() {
        anyhow::bail!("Autenticazione fallita (HTTP {})", response.status());
    }

    let auth: AuthResponse = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Risposta di autenticazione non valida: {}", e))?;

    Ok(auth.token)
}

