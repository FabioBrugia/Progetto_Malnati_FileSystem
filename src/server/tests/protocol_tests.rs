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
fn test_generate_success_response() {
    // Health endpoint must return protocol-level success JSON.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let app = awtest::init_service(App::new().route("/health", web::get().to(handlers::health))).await;

        let req = awtest::TestRequest::get().uri("/health").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("response JSON must be valid");

        assert_eq!(json["success"], true);
        assert_eq!(json["message"], "ok");
    });
}

#[test]
fn test_generate_error_response() {
    // Reading a missing file must produce protocol-compliant error status.
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

        let req = awtest::TestRequest::get().uri("/files/nope.txt").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        let msg: String = serde_json::from_slice(&body).expect("error body must be valid JSON string");
        assert_eq!(msg, "File not found");
    });
}

#[test]
fn test_generate_file_content_response() {
    // File read must return octet-stream payload with exact bytes.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("content.bin"), vec![10_u8, 20, 30])
            .expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::get().to(handlers::read_file)),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/files/content.bin").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("application/octet-stream"));

        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        assert_eq!(body.as_ref(), &[10_u8, 20, 30]);
    });
}

#[test]
fn test_generate_empty_response() {
    // HEAD metadata response must not include a body payload.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let dir = tempdir().expect("tempdir must be created");
        fs::write(dir.path().join("meta.txt"), "hello").expect("seed file write must succeed");

        let state = handlers::AppState {
            base_dir: dir.path().to_string_lossy().to_string(),
        };

        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .route("/files/{path:.*}", web::head().to(handlers::file_info)),
        )
        .await;

        let req = awtest::TestRequest::default()
            .method(actix_web::http::Method::HEAD)
            .uri("/files/meta.txt")
            .to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("content-length"));

        let body = to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        assert!(body.is_empty());
    });
}
