use actix_web::{web, App, HttpServer,};
use actix_web::middleware::Logger;
use actix_cors::Cors;
use std::fs;
mod handlers;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Init logging from RUST_LOG or default
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,actix_web=info");
    }
    env_logger::init();

    // Base dir fissa; host/port fissi
    let base_dir = "/tmp/remote_fs_test".to_string();

    println!("Avvio server Remote File System su http://0.0.0.0:9000 (base_dir: {})", base_dir);

    // Crea la directory base se non esiste
    fs::create_dir_all(&base_dir).unwrap();

    HttpServer::new(move || {
        let cors = Cors::permissive();
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .app_data(web::Data::new(handlers::AppState {
                base_dir: base_dir.clone(),
            }))
            .route("/", web::get().to(handlers::index))
            .route("/list/{path:.*}", web::get().to(handlers::list_directory))
            .route("/files/{path:.*}", web::get().to(handlers::read_file))
            .route("/files/{path:.*}", web::put().to(handlers::write_file))
            .route("/files/{path:.*}", web::head().to(handlers::file_info))
            .route("/mkdir/{path:.*}", web::post().to(handlers::create_directory))
            .route("/files/{path:.*}", web::delete().to(handlers::delete_file))
            .route("/rename", web::post().to(handlers::rename_entry))
            .route("/health", web::get().to(handlers::health))
    })
    .bind("0.0.0.0:9000")?
    .run()
    .await
}