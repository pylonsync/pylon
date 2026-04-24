//! Argon2id password hashing + verification.
//!
//! Kept tiny on purpose — no in-memory store, no plugin glue. Password
//! hashes live on the application's own entity (conventionally a
//! `passwordHash` column on `User`), so persistence is the same story
//! as every other row. Router endpoints under `/api/auth/password/*`
//! call these helpers to mint the hash + verify at login.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2, PasswordHash, PasswordVerifier,
};

/// Hash a password using Argon2id with a random salt. Returns a
/// PHC-format string carrying the algorithm, params, salt, and hash.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hash should succeed")
        .to_string()
}

/// Verify a password against an Argon2 PHC-format hash. Constant-time
/// comparison is handled internally by Argon2's `verify_password`.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// A PHC-format hash of a throwaway string — used to equalize response
/// timing when a login is attempted with an email that isn't registered.
/// Without this, `known-email + wrong-password` takes ~50ms (Argon2) and
/// `unknown-email` takes <1ms, letting an attacker enumerate the user
/// set by response time alone.
pub fn dummy_hash() -> &'static str {
    "$argon2id$v=19$m=19456,t=2,p=1$YWFhYWFhYWFhYWFhYWFhYQ$b3W/3pZzm6S8w5qYvJ8y3A"
}
