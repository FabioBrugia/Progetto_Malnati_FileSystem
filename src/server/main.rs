use actix_web::{App, HttpResponse, HttpServer, Responder, post, web};
use actix_web::middleware::Logger;
use actix_cors::Cors;
use serde::{Deserialize, Serialize};
use std::fs;
mod handlers;
mod auth;
use rand::Rng;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
mod auth_middleware;

use auth_middleware::{AuthMiddleware, TokenStore};

fn generate_token() -> String {
    let bytes: [u8; 32] = rand::thread_rng().gen();
    hex::encode(bytes)
}
const STORED_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$J5s7+n/0wmTc/efZmJLqqg$rc0OMVsF/iIwYKWAmoG+Ktar6C5Z9QTBe4HGJtHG70E";

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
    tokens: web::Data<TokenStore>,
    req: web::Json<AuthRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    if auth::verify_password(&req.password, STORED_PASSWORD_HASH) {
        let token = generate_token();
        tokens.lock().unwrap().insert(token.clone());
        Ok(HttpResponse::Ok().json(AuthResponse { token }))
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

    // Base dir fissa; host/port fissi
    let base_dir = "server_storage".to_string();

    println!("Avvio server Remote File System su http://0.0.0.0:8080 (base_dir: {})", base_dir);

    // Crea la directory base se non esiste
    fs::create_dir_all(&base_dir).unwrap();

    let tokens: TokenStore = Arc::new(Mutex::new(HashSet::new()));

    HttpServer::new(move || {
        let cors = Cors::default();
        App::new()
            .wrap(Logger::default())
            .wrap(AuthMiddleware {
                tokens: tokens.clone(),
            })
            .app_data(web::Data::new(
                handlers::AppState {
                    base_dir: base_dir.clone(),
                }
            ))
            .app_data(web::Data::new(tokens.clone()))
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
            .route("/health", web::get().to(handlers::health))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}