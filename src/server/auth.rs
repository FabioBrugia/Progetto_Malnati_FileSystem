use argon2::{
    Argon2, PasswordHasher, password_hash::{PasswordHash, PasswordVerifier, SaltString}
};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct JwtConfig {
    pub secret: String,
    pub expiration_seconds: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iat: usize,
}

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed_hash) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok(),
        Err(_) => false,
    }
}

pub fn create_jwt(subject: &str, config: &JwtConfig) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now().timestamp();
    let exp = now + config.expiration_seconds.max(1);

    let claims = Claims {
        sub: subject.to_string(),
        exp: exp as usize,
        iat: now as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.secret.as_bytes()),
    )
}

pub fn verify_jwt(token: &str, config: &JwtConfig) -> bool {
    let validation = Validation::new(Algorithm::HS256);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.secret.as_bytes()),
        &validation,
    )
    .is_ok()
}