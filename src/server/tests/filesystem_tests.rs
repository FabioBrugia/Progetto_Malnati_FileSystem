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
fn test_create_file_success() {
    // Creates a new file through PUT and verifies the file content on disk.
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
            .uri("/files/new.txt")
            .set_payload("hello")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let on_disk = fs::read_to_string(dir.path().join("new.txt"))
            .expect("created file must be readable");
        assert_eq!(on_disk, "hello");
    });
}

#[test]
fn test_create_file_already_exists() {
    // Verifies current server behavior: PUT on existing file truncates/overwrites.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let file_path = dir.path().join("existing.txt");
        fs::write(&file_path, "old").expect("seed file write must succeed");

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
            .uri("/files/existing.txt")
            .set_payload("new")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let on_disk = fs::read_to_string(file_path).expect("overwritten file must be readable");
        assert_eq!(on_disk, "new");
    });
}

#[test]
fn test_read_file_success() {
    // Reads a previously created file and validates binary response body.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("readme.txt"), b"abc123").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/files/readme.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        assert_eq!(body.as_ref(), b"abc123");
    });
}

#[test]
fn test_read_file_not_found() {
    // Missing files must return a NotFound protocol error.
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

        let req = awtest::TestRequest::get().uri("/files/missing.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    });
}

#[test]
fn test_write_file_success() {
    // Writes binary content and checks bytes_written metadata in JSON response.
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
            .uri("/files/data.bin")
            .set_payload(vec![1_u8, 2, 3, 4])
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("response JSON must be valid");

        assert_eq!(json["success"], true);
        assert_eq!(json["bytes_written"], 4);
    });
}

#[test]
fn test_write_file_overwrite() {
    // A second PUT on the same path must overwrite old bytes.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("same.txt"), "first").expect("seed file write must succeed");

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
            .uri("/files/same.txt")
            .set_payload("second")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let on_disk = fs::read_to_string(dir.path().join("same.txt"))
            .expect("updated file must be readable");
        assert_eq!(on_disk, "second");
    });
}

#[test]
fn test_delete_file_success() {
    // Deletes an existing file and verifies that it disappears from disk.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("trash.txt"), "x").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::delete().to(handlers::delete_file)),
        )
        .await;

        let req = awtest::TestRequest::delete().uri("/files/trash.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!dir.path().join("trash.txt").exists());
    });
}

#[test]
fn test_delete_file_not_found() {
    // Deleting a missing file must produce NotFound.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::delete().to(handlers::delete_file)),
        )
        .await;

        let req = awtest::TestRequest::delete().uri("/files/missing.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    });
}

#[test]
fn test_list_directory_contents() {
    // Lists root directory and verifies both file and subdirectory are reported.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("a.txt"), "a").expect("seed file write must succeed");
        fs::create_dir_all(dir.path().join("nested")).expect("seed directory creation must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/list/{path:.*}", web::get().to(handlers::list_directory)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/list/").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("response JSON must be valid");
        let entries = json["entries"].as_array().expect("entries must be an array");

        assert!(entries.iter().any(|e| e["name"] == "a.txt" && e["is_dir"] == false));
        assert!(entries.iter().any(|e| e["name"] == "nested" && e["is_dir"] == true));
    });
}
