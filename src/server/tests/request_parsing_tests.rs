use crate::handlers;
use actix_web::{
    http::StatusCode,
    test as awtest,
    web,
    App,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_parse_valid_create_request() {
    // Verifies that a valid create (mkdir) request path is parsed and executed.
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

        let req = awtest::TestRequest::post().uri("/mkdir/docs/newdir").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(dir.path().join("docs/newdir").is_dir());
    });
}

#[test]
fn test_parse_valid_read_request() {
    // Verifies read request path parsing for GET /files/{path}.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("in.txt"), "ok").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/files/in.txt").to_request();
        let resp = awtest::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    });
}

#[test]
fn test_parse_valid_write_request() {
    // Verifies write request path parsing for PUT /files/{path}.
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
            .uri("/files/dir/write.txt")
            .set_payload("payload")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(dir.path().join("dir/write.txt").exists());
    });
}

#[test]
fn test_parse_valid_delete_request() {
    // Verifies delete request path parsing for DELETE /files/{path}.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("delete_me.txt"), "x").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::delete().to(handlers::delete_file)),
        )
        .await;

        let req = awtest::TestRequest::delete()
            .uri("/files/delete_me.txt")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!dir.path().join("delete_me.txt").exists());
    });
}

#[test]
fn test_parse_invalid_command() {
    // Unsupported command shape in this router setup returns NotFound.
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

        let req = awtest::TestRequest::post().uri("/files/invalid.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    });
}

#[test]
fn test_parse_malformed_request() {
    // PATCH without required Content-Range header is a malformed request.
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
            .uri("/files/chunk.bin")
            .set_payload("abc")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    });
}

#[test]
fn test_parse_missing_parameters() {
    // Missing required JSON fields in rename request must fail parsing.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/rename", web::post().to(handlers::rename_entry)),
        )
        .await;

        let req = awtest::TestRequest::post()
            .uri("/rename")
            .insert_header(("content-type", "application/json"))
            .set_payload(r#"{"from":"a.txt"}"#)
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    });
}
