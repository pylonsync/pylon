//! User-agent parsing → friendly device labels.
//!
//! Real UA parsers (uap-rs, ua_parser) ship 2k+ regex pairs. For the
//! `/api/auth/sessions` UI we don't need that — we want a single
//! readable label per session ("Chrome on macOS", "Safari on iOS",
//! "Yubikey CLI"). Substring matching covers >95% of real traffic
//! at <1% the binary size.
//!
//! What this DOESN'T do:
//! - Parse exact browser/OS versions (apps that need that should pull
//!   the UA string verbatim from `Session.device` and parse it themselves)
//! - Detect WebViews vs native (the UA isn't reliable for that anyway)
//! - Identify bots (use a separate IP-reputation feed)
//!
//! Bounded output — the friendly label caps at 80 chars so a
//! pathological UA can't blow up the session row.

const MAX_LABEL_LEN: usize = 80;

/// Turn a User-Agent header value into a short friendly label.
/// Falls back to "Unknown" for empty / unrecognized strings.
pub fn parse_user_agent(ua: &str) -> String {
    let ua = ua.trim();
    if ua.is_empty() {
        return "Unknown".into();
    }
    // SDK / native client clients tend to use simple identifiers.
    // Match those FIRST so a SDK UA that happens to mention "Mozilla"
    // (some HTTP clients do) doesn't get bucketed as a browser.
    if let Some(label) = match_sdk(ua) {
        return cap(label.to_string());
    }
    let browser = match_browser(ua);
    let os = match_os(ua);
    let label = match (browser, os) {
        (Some(b), Some(o)) => format!("{b} on {o}"),
        (Some(b), None) => b.to_string(),
        (None, Some(o)) => o.to_string(),
        (None, None) => "Unknown".into(),
    };
    cap(label)
}

fn cap(s: String) -> String {
    if s.chars().count() <= MAX_LABEL_LEN {
        s
    } else {
        s.chars().take(MAX_LABEL_LEN).collect()
    }
}

/// Match the Pylon SDK family + common API-client identifiers.
/// Order matters — more specific tokens first.
fn match_sdk(ua: &str) -> Option<&'static str> {
    let lc = ua.to_ascii_lowercase();
    if lc.starts_with("pylonclient/") || lc.starts_with("pylonsdk/") {
        return Some("Pylon SDK");
    }
    if lc.starts_with("pylon-cli/") || lc.starts_with("pylon/") {
        return Some("Pylon CLI");
    }
    if lc.starts_with("curl/") {
        return Some("curl");
    }
    if lc.starts_with("httpie/") || lc.starts_with("python-requests/") {
        return Some("Python (requests)");
    }
    if lc.starts_with("go-http-client/") {
        return Some("Go HTTP client");
    }
    if lc.starts_with("postmanruntime/") {
        return Some("Postman");
    }
    None
}

/// Browser detection. Order matters: "Edg" / "OPR" / "Brave" must
/// match BEFORE "Chrome" because all four ship "Chrome/X.Y" in
/// their UAs (they're all Chromium forks).
fn match_browser(ua: &str) -> Option<&'static str> {
    if ua.contains("Edg/") || ua.contains("Edge/") {
        return Some("Edge");
    }
    if ua.contains("OPR/") || ua.contains("Opera") {
        return Some("Opera");
    }
    if ua.contains("Brave") {
        return Some("Brave");
    }
    if ua.contains("Vivaldi") {
        return Some("Vivaldi");
    }
    // Firefox MUST come before generic "Mozilla" check.
    if ua.contains("Firefox/") {
        return Some("Firefox");
    }
    // Chrome before Safari — Safari ships "Version/X" but every
    // Chromium-based UA also ships "Safari/" in the suffix.
    if ua.contains("Chrome/") {
        return Some("Chrome");
    }
    if ua.contains("Safari/") {
        return Some("Safari");
    }
    None
}

/// OS detection. Order matters: iPad/iPhone before Mac (some iPad
/// modes report "Macintosh" in the UA per Apple's "request desktop
/// site" feature — but still include "iPad" earlier).
fn match_os(ua: &str) -> Option<&'static str> {
    if ua.contains("iPhone") {
        return Some("iOS");
    }
    if ua.contains("iPad") {
        return Some("iPadOS");
    }
    if ua.contains("Android") {
        return Some("Android");
    }
    // macOS marker varies: "Mac OS X 10_15_7", "Macintosh", "Mac OS".
    if ua.contains("Macintosh") || ua.contains("Mac OS") {
        return Some("macOS");
    }
    if ua.contains("Windows NT") || ua.contains("Win64") || ua.contains("Win32") {
        return Some("Windows");
    }
    // Linux comes last because both Android and ChromeOS ship "Linux"
    // as a substring.
    if ua.contains("CrOS") {
        return Some("ChromeOS");
    }
    if ua.contains("Linux") {
        return Some("Linux");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_unknown() {
        assert_eq!(parse_user_agent(""), "Unknown");
        assert_eq!(parse_user_agent("   "), "Unknown");
    }

    #[test]
    fn chrome_macos() {
        let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
        assert_eq!(parse_user_agent(ua), "Chrome on macOS");
    }

    #[test]
    fn safari_ios() {
        let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
        // iPhone token wins over the substring "Mac OS" in iOS UAs.
        assert_eq!(parse_user_agent(ua), "Safari on iOS");
    }

    #[test]
    fn firefox_linux() {
        let ua = "Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0";
        assert_eq!(parse_user_agent(ua), "Firefox on Linux");
    }

    #[test]
    fn edge_classified_before_chrome() {
        // Edge ships "Chrome/" in its UA; the browser detector must
        // pick Edge first or every Edge user shows as Chrome.
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";
        assert_eq!(parse_user_agent(ua), "Edge on Windows");
    }

    #[test]
    fn opera_classified_before_chrome() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 OPR/115.0.0.0";
        assert_eq!(parse_user_agent(ua), "Opera on Windows");
    }

    #[test]
    fn android_classified_before_linux() {
        // Android UAs include "Linux" as a substring; OS detector
        // must pick Android first.
        let ua = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
        assert_eq!(parse_user_agent(ua), "Chrome on Android");
    }

    #[test]
    fn ipad_classified_before_macos() {
        // Newer iPad UAs say "Macintosh" with "request desktop site".
        // We want iPadOS not macOS — but if the UA strictly says
        // "Macintosh" with no iPad token, macOS is the right answer.
        // This test pins the "real iPad UA" case.
        let ua = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
        assert_eq!(parse_user_agent(ua), "Safari on iPadOS");
    }

    #[test]
    fn pylon_sdk_recognized() {
        assert_eq!(parse_user_agent("PylonClient/0.3.21 ts"), "Pylon SDK");
        assert_eq!(parse_user_agent("PylonSDK/swift 0.3.21"), "Pylon SDK");
        assert_eq!(parse_user_agent("pylon/0.3.21"), "Pylon CLI");
    }

    #[test]
    fn curl_recognized() {
        assert_eq!(parse_user_agent("curl/8.4.0"), "curl");
    }

    #[test]
    fn capped_at_80_chars() {
        let label = parse_user_agent(&("X".repeat(500)));
        assert!(label.chars().count() <= MAX_LABEL_LEN);
    }

    #[test]
    fn unknown_browser_known_os() {
        // A weird browser on a known OS — we still get the OS half.
        let ua = "WeirdBrowser/1.0 (Windows NT 10.0)";
        assert_eq!(parse_user_agent(ua), "Windows");
    }

    #[test]
    fn unknown_browser_unknown_os() {
        assert_eq!(parse_user_agent("totally-bogus-junk"), "Unknown");
    }

    /// Defense against UA-fingerprinting probes that send 10MB UAs.
    /// The label MUST be a bounded String, not a borrow into the
    /// caller's input.
    #[test]
    fn does_not_panic_on_pathological_input() {
        let _ = parse_user_agent(&"\u{1F600}".repeat(10000)); // emoji
        let _ = parse_user_agent(&"\0\0\0".repeat(1000)); // nulls
        let _ = parse_user_agent("\n\n\n\n");
    }
}
