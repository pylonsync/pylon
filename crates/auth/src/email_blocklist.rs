//! Disposable / throwaway email domain blocklist.
//!
//! Refuses signups (password/register, magic-link/send, OAuth callback)
//! when the email's domain is a known throwaway provider. Blocks at the
//! identity-creation boundary instead of at downstream resource-creation
//! sites, so apps don't have to re-litigate this in every gate.
//!
//! The built-in list is curated to ~80 well-known operators (mailinator,
//! 10minutemail, guerrillamail, yopmail, etc.). The full public lists run
//! to 100k+ entries and have meaningful false-positive rates; we trade
//! coverage for low false positives.
//!
//! ## Configuration
//!
//! - `PYLON_EMAIL_BLOCKLIST_DISABLED=1` — turn the check off entirely.
//!   Default off (i.e. blocklist active). Set this if you're running a
//!   service where transient inboxes are a legitimate sign-up pattern
//!   (e.g. a public testing tool).
//! - `PYLON_EMAIL_BLOCKLIST_EXTRA="dom1.com,dom2.com"` — comma-separated
//!   additions to the built-in list. Useful for blocking abuse you've
//!   actually seen without waiting for a release.
//!
//! ## Programmatic use
//!
//! Apps using pylon-auth as a library (or wrapping the routes) can call
//! [`is_disposable_email`] directly to apply the same check at custom
//! check points (e.g. invite acceptance, plan upgrade flows).

use std::collections::HashSet;
use std::sync::OnceLock;

/// Returns true if the email's domain is on the disposable blocklist
/// AND the blocklist isn't disabled via env. Safe to call hot — the
/// HashSet construction is memoized via OnceLock.
///
/// Domain comparison is case-insensitive. Returns false for malformed
/// addresses (no `@`, empty domain) — defensive: don't block valid
/// users by mishandling edge cases.
pub fn is_disposable_email(email: &str) -> bool {
    if std::env::var("PYLON_EMAIL_BLOCKLIST_DISABLED").as_deref() == Ok("1") {
        return false;
    }
    let Some(domain) = extract_domain(email) else {
        return false;
    };
    blocklist().contains(domain.as_str())
}

/// Lower-cased domain part of an email, or None if malformed.
pub fn extract_domain(email: &str) -> Option<String> {
    let at = email.rfind('@')?;
    if at == 0 || at == email.len() - 1 {
        return None;
    }
    let domain = email[at + 1..].trim().to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    Some(domain)
}

fn blocklist() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(build_blocklist)
}

fn build_blocklist() -> HashSet<String> {
    let mut set: HashSet<String> = BUILTIN.iter().map(|s| s.to_ascii_lowercase()).collect();
    if let Ok(extra) = std::env::var("PYLON_EMAIL_BLOCKLIST_EXTRA") {
        for entry in extra.split(',') {
            let d = entry.trim().to_ascii_lowercase();
            if !d.is_empty() {
                set.insert(d);
            }
        }
    }
    set
}

/// Built-in blocklist. Curated to known throwaway operators that show
/// up in real abuse signups; intentionally narrower than the 100k-entry
/// public lists to keep false positives low.
const BUILTIN: &[&str] = &[
    // Mailinator family
    "mailinator.com",
    "mailinator.net",
    "mailinator.org",
    "mailinator2.com",
    "binkmail.com",
    "bobmail.info",
    "chammy.info",
    "devnullmail.com",
    "letthemeatspam.com",
    "mailin8r.com",
    "mailnesia.com",
    "reallymymail.com",
    "sogetthis.com",
    "spamherelots.com",
    "superrito.com",
    "thisisnotmyrealemail.com",
    "tradermail.info",
    "veryrealemail.com",
    "zippymail.info",
    // 10minutemail family
    "10minutemail.com",
    "10minutemail.net",
    "10minutemail.org",
    "10minutemail.de",
    "20minutemail.com",
    "30minutemail.com",
    "33mail.com",
    // Guerrilla Mail
    "guerrillamail.com",
    "guerrillamail.net",
    "guerrillamail.org",
    "guerrillamail.biz",
    "guerrillamail.de",
    "guerrillamailblock.com",
    "sharklasers.com",
    "grr.la",
    "pokemail.net",
    "spam4.me",
    // Temp Mail
    "tempmail.com",
    "tempmail.net",
    "tempmail.org",
    "temp-mail.org",
    "temp-mail.io",
    "tempmailaddress.com",
    "tempmailo.com",
    "temp-mail.com",
    "linshiyouxiang.net",
    // Yopmail
    "yopmail.com",
    "yopmail.fr",
    "yopmail.net",
    // Other commonly-seen disposables
    "throwawaymail.com",
    "trashmail.com",
    "trashmail.de",
    "trashmail.net",
    "trashmail.org",
    "getairmail.com",
    "getnada.com",
    "nada.email",
    "dispostable.com",
    "emailondeck.com",
    "fakeinbox.com",
    "fakemail.fr",
    "mintemail.com",
    "mt2014.com",
    "mt2015.com",
    "mvrht.com",
    "mytemp.email",
    "mytrashmail.com",
    "sneakemail.com",
    "spambog.com",
    "spambox.us",
    "tempinbox.com",
    "tempinbox.co.uk",
    "trashmail.ws",
    "e4ward.com",
    "emltmp.com",
    "jetable.org",
    "jetable.fr.nf",
    "mailcatch.com",
    "mailexpire.com",
    "mailfreeonline.com",
    "mailmoat.com",
    "mailshell.com",
    "mailtothis.com",
    "mailzilla.org",
    "maildrop.cc",
    "objectmail.com",
    "proxymail.eu",
    "rcpt.at",
    "recode.me",
    "rmqkr.net",
    "safetymail.info",
    "sandelf.de",
    "spam.la",
    "spamcero.com",
    "spamfree24.org",
    "spamhereplease.com",
    "spamhole.com",
    "spaml.com",
    "spamspot.com",
    "speed.1s.fr",
    "suremail.info",
    "tagyourself.com",
    "talkinator.com",
    "tilien.com",
    "trbvm.com",
    "twkly.ml",
    "willhackforfood.biz",
    "willselfdestruct.com",
    "yapped.net",
    "yuurok.com",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Wipe env state between tests so PYLON_EMAIL_BLOCKLIST_DISABLED
    /// from one test doesn't bleed into another. The OnceLock is
    /// process-global and we don't reset it here — tests that depend
    /// on `_EXTRA` would see stale state if they ran first. Keep
    /// EXTRA-dependent assertions out of the unit suite.
    fn clear_env() {
        std::env::remove_var("PYLON_EMAIL_BLOCKLIST_DISABLED");
    }

    #[test]
    fn blocks_known_throwaway() {
        clear_env();
        assert!(is_disposable_email("foo@mailinator.com"));
        assert!(is_disposable_email("FOO@MAILINATOR.COM"));
        assert!(is_disposable_email("a@10minutemail.com"));
        assert!(is_disposable_email("user@yopmail.fr"));
    }

    #[test]
    fn allows_real_providers() {
        clear_env();
        assert!(!is_disposable_email("alice@gmail.com"));
        assert!(!is_disposable_email("bob@protonmail.com"));
        assert!(!is_disposable_email("carol@stack0.dev"));
        assert!(!is_disposable_email("dave@pylonsync.com"));
    }

    #[test]
    fn malformed_addresses_pass_through() {
        clear_env();
        // No @, empty parts — neither blocked nor errored.
        assert!(!is_disposable_email("not-an-email"));
        assert!(!is_disposable_email("@nodomain.com"));
        assert!(!is_disposable_email("nolocal@"));
        assert!(!is_disposable_email(""));
    }

    // PYLON_EMAIL_BLOCKLIST_DISABLED behavior is intentionally not unit
    // tested — `cargo test` runs threads in parallel and env vars are
    // process-global, so flipping it from one test races with reads in
    // others. The env check itself is a one-line guard at the top of
    // `is_disposable_email`; covered implicitly by ops integration.

    #[test]
    fn extract_domain_handles_edges() {
        assert_eq!(extract_domain("a@b.com").as_deref(), Some("b.com"));
        assert_eq!(
            extract_domain("Mixed@CASE.ExAmPlE").as_deref(),
            Some("case.example")
        );
        assert_eq!(extract_domain("noatsign").as_deref(), None);
        assert_eq!(extract_domain("@nolocal").as_deref(), None);
        assert_eq!(extract_domain("nodomain@").as_deref(), None);
    }
}
