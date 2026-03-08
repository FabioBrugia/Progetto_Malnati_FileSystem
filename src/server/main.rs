use actix_web::{App, HttpResponse, HttpServer, post, web};
use actix_web::middleware::Logger;
use actix_cors::Cors;
use serde::{Deserialize, Serialize};
use std::fs;
mod handlers;
mod auth;
mod auth_middleware;
#[cfg(test)]
mod tests;

use auth_middleware::AuthMiddleware;
const STORED_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$I1XtoiH6Ni2CbrR3GWRlXA$KK/6iTuzFPa8PdFd7CFtkRKEH/0ZpNV0hPTZeiKH3BQ";

#[derive(Deserialize)]
struct AuthRequest {
    password: String,
}

#[derive(Serialize)]
struct AuthResponse {
    token: String,
}

#[post("/auth")]
async fn authenticate(
    jwt_config: web::Data<auth::JwtConfig>,
    req: web::Json<AuthRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    if auth::verify_password(&req.password, STORED_PASSWORD_HASH) {
        match auth::create_jwt("client", &jwt_config) {
            Ok(token) => Ok(HttpResponse::Ok().json(AuthResponse { token })),
            Err(_) => Ok(HttpResponse::InternalServerError().finish()),
        }
    } else{
        Ok(HttpResponse::Unauthorized().finish())
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Init logging from RUST_LOG or default
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,actix_web=info");
    }
    env_logger::init();

    // Consente override via env per test/integrazione.
    let base_dir = std::env::var("SERVER_BASE_DIR").unwrap_or_else(|_| "server_storage".to_string());
    let bind_addr = std::env::var("SERVER_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    println!(
        "Avvio server Remote File System su http://{} (base_dir: {})",
        bind_addr, base_dir
    );

    // Crea la directory base se non esiste
    fs::create_dir_all(&base_dir).unwrap();

    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "change-me-in-production".to_string());
    let jwt_expiration_seconds = std::env::var("JWT_EXPIRATION_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(3600);
    let jwt_config = auth::JwtConfig {
        secret: jwt_secret,
        expiration_seconds: jwt_expiration_seconds,
    };

    HttpServer::new(move || {
        let cors = Cors::default();
        App::new()
            .wrap(Logger::default())
            .wrap(AuthMiddleware {
                jwt_config: jwt_config.clone(),
            })
            .app_data(web::Data::new(
                handlers::AppState {
                    base_dir: base_dir.clone(),
                }
            ))
            .app_data(web::Data::new(jwt_config.clone()))
            .service(authenticate)
            .wrap(cors)
            .route("/", web::get().to(handlers::index))
            .route("/list/{path:.*}", web::get().to(handlers::list_directory))
            .route("/files/{path:.*}", web::get().to(handlers::read_file))
            .route("/files/{path:.*}", web::put().to(handlers::write_file))
            .route("/files/{path:.*}", web::patch().to(handlers::patch_file))
            .route("/files/{path:.*}", web::head().to(handlers::file_info))
            .route("/mkdir/{path:.*}", web::post().to(handlers::create_directory))
            .route("/files/{path:.*}", web::delete().to(handlers::delete_file))
            .route("/rename", web::post().to(handlers::rename_entry))
            .route("/attrs/{path:.*}", web::patch().to(handlers::set_attrs))
            .route("/health", web::get().to(handlers::health))
    })
    .bind(&bind_addr)?
    .run()
    .await
}