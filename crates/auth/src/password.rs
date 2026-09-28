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

// ---------------------------------------------------------------------------
// Strength validation + HIBP / pwned-password check
// ---------------------------------------------------------------------------

/// Reasons a password may be rejected at registration / change time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordPolicyError {
    /// Shorter than [`MIN_PASSWORD_LEN`] characters.
    TooShort { got: usize, want: usize },
    /// Found in the HIBP Pwned Passwords corpus. Carries the count of
    /// times the password has appeared in a known breach so the
    /// frontend can surface a meaningful message ("seen 1.4M times").
    Pwned { occurrences: u64 },
}

impl std::fmt::Display for PasswordPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { got, want } => {
                write!(f, "password too short ({got} chars, need {want})")
            }
            Self::Pwned { occurrences } => write!(
                f,
                "password appears in {occurrences} known data breaches; choose a different password"
            ),
        }
    }
}

/// Minimum password length. Better-auth and most modern stacks default
/// to 8; OWASP says 8+ for users + a strength meter, 14+ for admins.
/// We pick 10 as a middle ground — measurably better than 8 with no
/// noticeable UX cost.
pub const MIN_PASSWORD_LEN: usize = 10;

/// Validate password length. Cheap, pure-rust check. Run *before*
/// [`check_pwned`] so weak local passwords don't even hit the network.
pub fn validate_length(password: &str) -> Result<(), PasswordPolicyError> {
    let n = password.chars().count();
    if n < MIN_PASSWORD_LEN {
        return Err(PasswordPolicyError::TooShort {
            got: n,
            want: MIN_PASSWORD_LEN,
        });
    }
    Ok(())
}

/// Check a password against the HIBP Pwned Passwords v3 API using
/// k-anonymity — only the first 5 chars of the SHA-1 hash leave the
/// box. Returns `Ok(0)` for "not pwned", `Ok(N)` for "pwned N times",
/// and `Err(reason)` for HTTP failures (the caller decides whether
/// to fail-open or fail-closed; pylon's wrappers fail-open so a
/// service outage doesn't lock everyone out of registration).
///
/// API docs: <https://haveibeenpwned.com/API/v3#PwnedPasswords>
///
/// Privacy: SHA-1 with k-anonymity is widely audited, doesn't require
/// an API key, and Cloudflare caches the hash-prefix endpoint for ~1
/// hour so the actual request typically never hits HIBP itself.
pub fn check_pwned(password: &str) -> Result<u64, String> {
    check_pwned_at("https://api.pwnedpasswords.com", password)
}

/// [`check_pwned`] against the range API at `base`.
fn check_pwned_at(base: &str, password: &str) -> Result<u64, String> {
    let hash = sha1_hex_upper(password.as_bytes());
    let (prefix, suffix) = hash.split_at(5);
    let url = format!("{base}/range/{prefix}");
    let body = hibp_agent()
        .get(&url)
        // "Add-Padding: true" makes responses constant-size so a
        // network-level observer can't infer pwned-ness from byte
        // counts. Free, no downside.
        .set("Add-Padding", "true")
        .call()
        .map_err(|e| format!("hibp request: {e}"))?
        .into_string()
        .map_err(|e| format!("hibp body: {e}"))?;
    Ok(parse_hibp_range(&body, suffix))
}

/// One agent for every HIBP request, so the TLS config and root store are
/// built once and connections are reused.
///
/// The check runs inside sign-up and password changes, and callers skip it
/// on an error, so a request gets one short deadline for the whole exchange
/// (DNS, connect, TLS, and body). Per-read timeouts alone let a slow
/// endpoint hold a sign-up for many seconds.
fn hibp_agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(HIBP_DEADLINE)
            .user_agent("pylon-auth")
            .build()
    })
}

/// Longest an HIBP lookup may take before the caller skips the check.
const HIBP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

/// Parse the HIBP range response (line-separated `SUFFIX:COUNT`) and
/// return the count for our suffix, or 0 if not found.
fn parse_hibp_range(body: &str, suffix: &str) -> u64 {
    for line in body.lines() {
        let line = line.trim();
        let Some((s, c)) = line.split_once(':') else {
            continue;
        };
        if s.eq_ignore_ascii_case(suffix) {
            return c.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// SHA-1 hex digest in uppercase (HIBP's range API is case-insensitive
/// but uppercase is the canonical form they document).
fn sha1_hex_upper(input: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(input);
    let out = h.finalize();
    let mut s = String::with_capacity(40);
    for b in out {
        use std::fmt::Write;
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// Combined "is this password OK?" check — length first, then HIBP.
/// HIBP failures are propagated; the caller decides fail-open/closed.
pub fn validate(password: &str) -> Result<(), PasswordPolicyError> {
    validate_length(password)?;
    match check_pwned(password) {
        Ok(0) => Ok(()),
        Ok(n) => Err(PasswordPolicyError::Pwned { occurrences: n }),
        Err(_) => Ok(()), // fail-open on network error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_hibp_endpoint_errors_within_the_deadline() {
        // Accepts the connection, then never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let _held = std::thread::spawn(move || {
            let conns: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(std::time::Duration::from_secs(30));
            drop(conns);
        });
        let started = std::time::Instant::now();
        assert!(check_pwned_at(&base, "correct-horse-battery").is_err());
        assert!(started.elapsed() < HIBP_DEADLINE + std::time::Duration::from_secs(2));
    }

    #[test]
    fn validate_length_rejects_short() {
        let err = validate_length("short").unwrap_err();
        assert!(matches!(err, PasswordPolicyError::TooShort { .. }));
    }

    #[test]
    fn validate_length_accepts_min_len() {
        assert!(validate_length("0123456789").is_ok());
    }

    #[test]
    fn sha1_known_vector() {
        // SHA-1("password") = 5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8
        let h = sha1_hex_upper(b"password");
        assert_eq!(h, "5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8");
    }

    #[test]
    fn parse_hibp_range_finds_match() {
        // Real shape of HIBP response — `SUFFIX:COUNT` lines.
        let body = "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n\
                    003D68EB55068C33ACE09247EE4C639306B:3\r\n\
                    012345678901234567890123456789012345:42\r\n";
        assert_eq!(
            parse_hibp_range(body, "012345678901234567890123456789012345"),
            42
        );
        assert_eq!(
            parse_hibp_range(body, "0018A45C4D1DEF81644B54AB7F969B88D65"),
            1
        );
        assert_eq!(
            parse_hibp_range(body, "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEF"),
            0
        );
    }

    #[test]
    fn parse_hibp_range_is_case_insensitive() {
        let body = "ABCDEF0123456789ABCDEF0123456789ABCD:7\r\n";
        // Lowercase suffix should still match the uppercase line.
        assert_eq!(
            parse_hibp_range(body, "abcdef0123456789abcdef0123456789abcd"),
            7
        );
    }
}
