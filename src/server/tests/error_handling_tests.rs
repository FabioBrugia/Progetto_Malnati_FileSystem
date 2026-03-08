use crate::handlers;
use actix_web::{
    http::StatusCode,
    test as awtest,
    web,
    App,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

#[test]
fn test_invalid_file_path() {
    // Path traversal attempts must be rejected during path validation.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/files/../escape.txt").to_request();
        let resp = awtest::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    });
}

#[test]
fn test_permission_denied_error() {
    // Write attempts in a non-writable directory must map to Forbidden.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let protected_dir = dir.path().join("protected");
        fs::create_dir_all(&protected_dir).expect("protected dir creation must succeed");

        let mut perms = fs::metadata(&protected_dir)
            .expect("metadata must be available")
            .permissions();
        perms.set_mode(0o500);
        fs::set_permissions(&protected_dir, perms).expect("must set non-writable mode");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::put().to(handlers::write_file)),
        )
        .await;

        let req = awtest::TestRequest::put()
            .uri("/files/protected/denied.txt")
            .set_payload("data")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        // Restore permissions for reliable temporary directory cleanup.
        let mut restore = fs::metadata(&protected_dir)
            .expect("metadata must be available")
            .permissions();
        restore.set_mode(0o700);
        fs::set_permissions(&protected_dir, restore).expect("permissions restore must succeed");

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    });
}

#[test]
fn test_invalid_request_format() {
    // Invalid Content-Range format must be reported as BadRequest.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::patch().to(handlers::patch_file)),
        )
        .await;

        let req = awtest::TestRequest::patch()
            .uri("/files/bad.bin")
            .insert_header(("Content-Range", "invalid-range"))
            .set_payload("x")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    });
}

#[test]
fn test_internal_server_error() {
    // Unreadable directory in list operation currently maps to InternalServerError.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let blocked_dir = dir.path().join("blocked");
        fs::create_dir_all(&blocked_dir).expect("blocked dir creation must succeed");

        let mut perms = fs::metadata(&blocked_dir)
            .expect("metadata must be available")
            .permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&blocked_dir, perms).expect("must set unreadable mode");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/list/{path:.*}", web::get().to(handlers::list_directory)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/list/blocked").to_request();
        let resp = awtest::call_service(&app, req).await;

        // Restore permissions for reliable temporary directory cleanup.
        let mut restore = fs::metadata(&blocked_dir)
            .expect("metadata must be available")
            .permissions();
        restore.set_mode(0o700);
        fs::set_permissions(&blocked_dir, restore).expect("permissions restore must succeed");

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    });
}
