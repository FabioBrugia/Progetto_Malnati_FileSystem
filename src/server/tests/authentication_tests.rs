use crate::auth;
use crate::auth_middleware::AuthMiddleware;
use actix_web::{
    http::StatusCode,
    test as awtest,
    web,
    App,
    HttpResponse,
};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;

#[derive(Serialize)]
struct ExpiredClaims {
    sub: String,
    exp: usize,
    iat: usize,
}

#[test]
fn test_valid_authentication() {
    // Password hash verification must succeed with the original plaintext.
    let password = "correct-horse-battery-staple";
    let hash = auth::hash_password(password);
    assert!(auth::verify_password(password, &hash));
}

#[test]
fn test_invalid_token() {
    // Random/non-JWT strings must fail token verification.
    let cfg = auth::JwtConfig {
        secret: "secret-key".to_string(),
        expiration_seconds: 3600,
    };
    assert!(!auth::verify_jwt("not-a-valid-token", &cfg));
}

#[test]
fn test_expired_token() {
    // Tokens with expired exp claim must be rejected.
    let cfg = auth::JwtConfig {
        secret: "secret-key".to_string(),
        expiration_seconds: 3600,
    };

    let expired = ExpiredClaims {
        sub: "client".to_string(),
        exp: 1,
        iat: 1,
    };

    let token = encode(
        &Header::default(),
        &expired,
        &EncodingKey::from_secret(cfg.secret.as_bytes()),
    )
    .expect("token encoding must succeed");

    assert!(!auth::verify_jwt(&token, &cfg));
}

#[test]
fn test_request_without_authentication() {
    // Middleware must block protected endpoints when Authorization header is missing.
    let rt = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    rt.block_on(async {
        let cfg = auth::JwtConfig {
            secret: "secret-key".to_string(),
            expiration_seconds: 3600,
        };

        let app = awtest::init_service(
            App::new()
                .wrap(AuthMiddleware {
                    jwt_config: cfg.clone(),
                })
                .route("/health", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;

        let req = awtest::TestRequest::get().uri("/health").to_request();
        let resp = awtest::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    });
}
