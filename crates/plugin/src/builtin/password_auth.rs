use std::collections::HashMap;
use std::sync::Mutex;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, rand_core::OsRng};

use crate::Plugin;
use agentdb_auth::AuthContext;

/// A stored password entry.
#[derive(Debug, Clone)]
struct PasswordEntry {
    user_id: String,
    email: String,
    /// Argon2 PHC-format hash string (includes salt, algorithm, and parameters).
    hash: String,
}

/// Password auth plugin. Stores hashed passwords using Argon2id.
///
/// Passwords are hashed with Argon2id (the recommended variant for password
/// hashing). The hash output is a PHC-format string that embeds the salt,
/// algorithm, memory/time parameters, and hash value.
pub struct PasswordAuthPlugin {
    entries: Mutex<HashMap<String, PasswordEntry>>,
}

impl PasswordAuthPlugin {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new user with email + password.
    pub fn register(&self, email: &str, password: &str, user_id: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        if entries.contains_key(email) {
            return Err("Email already registered".into());
        }

        let hash = hash_password(password);

        entries.insert(email.to_string(), PasswordEntry {
            user_id: user_id.to_string(),
            email: email.to_string(),
            hash,
        });

        Ok(())
    }

    /// Verify email + password. Returns the user_id if valid.
    /// Argon2's verify_password handles constant-time comparison internally.
    pub fn verify(&self, email: &str, password: &str) -> Option<String> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(email)?;
        if verify_password(password, &entry.hash) {
            Some(entry.user_id.clone())
        } else {
            None
        }
    }

    /// Change a user's password.
    pub fn change_password(&self, email: &str, old_password: &str, new_password: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(email).ok_or("User not found")?;

        if !verify_password(old_password, &entry.hash) {
            return Err("Incorrect password".into());
        }

        entry.hash = hash_password(new_password);
        Ok(())
    }

    /// Check if an email is registered.
    pub fn is_registered(&self, email: &str) -> bool {
        self.entries.lock().unwrap().contains_key(email)
    }

    /// Reset password (admin/magic-code flow — no old password needed).
    pub fn reset_password(&self, email: &str, new_password: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(email).ok_or("User not found")?;
        entry.hash = hash_password(new_password);
        Ok(())
    }
}

impl Plugin for PasswordAuthPlugin {
    fn name(&self) -> &str {
        "password-auth"
    }
}

/// Hash a password using Argon2id with a random salt.
///
/// Returns a PHC-format string that includes the algorithm, version,
/// parameters, salt, and hash — everything needed for verification.
fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .expect("Argon2 hash should succeed")
        .to_string()
}

/// Verify a password against an Argon2 PHC-format hash string.
///
/// Argon2's verify_password performs constant-time comparison internally,
/// so no separate constant_time_eq is needed.
fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::PasswordHash;
    let parsed = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_verify() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "password123", "user-1").unwrap();

        let user_id = plugin.verify("alice@test.com", "password123").unwrap();
        assert_eq!(user_id, "user-1");
    }

    #[test]
    fn wrong_password_rejected() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "password123", "user-1").unwrap();

        assert!(plugin.verify("alice@test.com", "wrong").is_none());
    }

    #[test]
    fn unknown_email_rejected() {
        let plugin = PasswordAuthPlugin::new();
        assert!(plugin.verify("nobody@test.com", "password").is_none());
    }

    #[test]
    fn duplicate_email_rejected() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "pass1", "user-1").unwrap();
        let result = plugin.register("alice@test.com", "pass2", "user-2");
        assert!(result.is_err());
    }

    #[test]
    fn change_password() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "old-pass", "user-1").unwrap();

        plugin.change_password("alice@test.com", "old-pass", "new-pass").unwrap();

        assert!(plugin.verify("alice@test.com", "old-pass").is_none());
        assert!(plugin.verify("alice@test.com", "new-pass").is_some());
    }

    #[test]
    fn change_password_wrong_old() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "password", "user-1").unwrap();

        let result = plugin.change_password("alice@test.com", "wrong", "new");
        assert!(result.is_err());
    }

    #[test]
    fn reset_password() {
        let plugin = PasswordAuthPlugin::new();
        plugin.register("alice@test.com", "old-pass", "user-1").unwrap();

        plugin.reset_password("alice@test.com", "reset-pass").unwrap();
        assert!(plugin.verify("alice@test.com", "reset-pass").is_some());
    }

    #[test]
    fn is_registered() {
        let plugin = PasswordAuthPlugin::new();
        assert!(!plugin.is_registered("alice@test.com"));
        plugin.register("alice@test.com", "pass", "user-1").unwrap();
        assert!(plugin.is_registered("alice@test.com"));
    }

    #[test]
    fn hash_is_phc_format() {
        let h = hash_password("test-password");
        // PHC format starts with "$argon2"
        assert!(h.starts_with("$argon2"), "Expected PHC format, got: {}", h);
    }

    #[test]
    fn same_password_different_hashes() {
        // Each call generates a new random salt, so hashes differ.
        let h1 = hash_password("password");
        let h2 = hash_password("password");
        assert_ne!(h1, h2);
        // But both verify correctly.
        assert!(verify_password("password", &h1));
        assert!(verify_password("password", &h2));
    }
}
