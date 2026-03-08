use crate::handlers;
use actix_web::{
    body::to_bytes,
    http::StatusCode,
    test as awtest,
    web,
    App,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_execute_create_command() {
    // Executes create directory command and verifies command routing to mkdir handler.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/mkdir/{path:.*}", web::post().to(handlers::create_directory)),
        )
        .await;

        let req = awtest::TestRequest::post().uri("/mkdir/x/y").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(dir.path().join("x/y").is_dir());
    });
}

#[test]
fn test_execute_read_command() {
    // Executes read command through dispatcher route and verifies returned content.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("doc.txt"), "read-ok").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/files/doc.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        assert_eq!(body.as_ref(), b"read-ok");
    });
}

#[test]
fn test_execute_write_command() {
    // Executes write command and verifies interaction with filesystem state.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
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
            .uri("/files/out.txt")
            .set_payload("written")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let on_disk = fs::read_to_string(dir.path().join("out.txt"))
            .expect("written file must be readable");
        assert_eq!(on_disk, "written");
    });
}

#[test]
fn test_execute_delete_command() {
    // Executes delete command and confirms the file is removed from storage.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("gone.txt"), "delete-me").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::delete().to(handlers::delete_file)),
        )
        .await;

        let req = awtest::TestRequest::delete().uri("/files/gone.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!dir.path().join("gone.txt").exists());
    });
}

#[test]
fn test_execute_unknown_command() {
    // Unsupported command method is not matched by this route table and returns NotFound.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file))
                .route("/files/{path:.*}", web::put().to(handlers::write_file))
                .route("/files/{path:.*}", web::delete().to(handlers::delete_file)),
        )
        .await;

        let req = awtest::TestRequest::post().uri("/files/unknown.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    });
}
