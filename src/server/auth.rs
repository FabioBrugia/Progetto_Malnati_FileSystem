use argon2::{
    Argon2, PasswordHasher, password_hash::{PasswordHash, PasswordVerifier, SaltString}
};
use rand_core::OsRng;

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let parsed_hash = PasswordHash::new(stored_hash).unwrap();
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}