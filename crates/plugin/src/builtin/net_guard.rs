/// Network security utilities for preventing SSRF attacks.
///
/// Provides a private/reserved IP blocklist check that should be called before
/// any outbound network connection (webhooks, SMTP, etc.) to prevent
/// Server-Side Request Forgery.

/// Returns `true` if the given host (with optional `:port` suffix) resolves to
/// a private, loopback, link-local, or otherwise reserved IP address range.
///
/// This blocks:
/// - `localhost`, `127.x.x.x`, `::1`, `0.0.0.0` (loopback/unspecified)
/// - `10.0.0.0/8` (RFC 1918)
/// - `172.16.0.0/12` (RFC 1918)
/// - `192.168.0.0/16` (RFC 1918)
/// - `169.254.0.0/16` (link-local, includes AWS metadata at 169.254.169.254)
/// - `0.0.0.0/8` (current network)
/// - `::1`, `::0`, `::` (IPv6 loopback/unspecified)
/// - `::ffff:x.x.x.x` (IPv4-mapped IPv6)
/// - `fc00::/7` (IPv6 Unique Local)
/// - `fe80::/10` (IPv6 Link-Local)
pub fn is_private_ip(host: &str) -> bool {
    // Extract the host portion, handling IPv6 brackets and port suffixes.
    let clean_host = if host.starts_with('[') {
        // Bracketed IPv6: [::1]:port or [::1]
        host.trim_start_matches('[').split(']').next().unwrap_or(host)
    } else if host.contains("::") || host.matches(':').count() > 1 {
        // Bare IPv6 without brackets (multiple colons means IPv6, not host:port)
        host
    } else {
        // IPv4 or hostname — split on last colon for port
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };

    // Check well-known hostnames.
    if clean_host == "localhost" || clean_host == "0.0.0.0" {
        return true;
    }

    // Check IPv6 loopback and unspecified.
    if clean_host == "::1" || clean_host == "::0" || clean_host == "::" {
        return true;
    }

    // IPv4-mapped IPv6: ::ffff:127.0.0.1
    if let Some(mapped) = clean_host.strip_prefix("::ffff:") {
        return is_private_ipv4(mapped);
    }

    // IPv6 Unique Local (fc00::/7) — covers fc00:: through fdff::
    if clean_host.starts_with("fc") || clean_host.starts_with("fd") {
        return true;
    }

    // IPv6 Link-Local (fe80::/10)
    if clean_host.starts_with("fe80") {
        return true;
    }

    // Check IPv4
    is_private_ipv4(clean_host)
}

/// Returns `true` if the given IPv4 address string is in a private/reserved range.
fn is_private_ipv4(host: &str) -> bool {
    let octets: Vec<u8> = host.split('.').filter_map(|s| s.parse().ok()).collect();
    if octets.len() == 4 {
        match (octets[0], octets[1]) {
            (127, _) => true,       // 127.0.0.0/8 loopback
            (10, _) => true,        // 10.0.0.0/8
            (172, 16..=31) => true,  // 172.16.0.0/12
            (192, 168) => true,      // 192.168.0.0/16
            (169, 254) => true,      // 169.254.0.0/16 (link-local + AWS metadata)
            (0, _) => true,          // 0.0.0.0/8
            _ => false,
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Loopback and special addresses ---

    #[test]
    fn blocks_localhost() {
        assert!(is_private_ip("localhost"));
        assert!(is_private_ip("localhost:8080"));
    }

    #[test]
    fn blocks_loopback_127() {
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("127.0.0.1:80"));
        assert!(is_private_ip("127.255.255.255"));
        assert!(is_private_ip("127.0.0.2"));
    }

    #[test]
    fn blocks_ipv6_loopback() {
        assert!(is_private_ip("::1"));
        assert!(is_private_ip("[::1]:8080"));
    }

    #[test]
    fn blocks_ipv6_unspecified() {
        assert!(is_private_ip("::0"));
        assert!(is_private_ip("::"));
    }

    #[test]
    fn blocks_unspecified() {
        assert!(is_private_ip("0.0.0.0"));
        assert!(is_private_ip("0.0.0.0:443"));
    }

    // --- RFC 1918 private ranges ---

    #[test]
    fn blocks_10_network() {
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("10.0.0.1:9090"));
    }

    #[test]
    fn blocks_172_16_network() {
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("172.20.0.1:443"));
    }

    #[test]
    fn allows_172_outside_range() {
        assert!(!is_private_ip("172.15.0.1"));
        assert!(!is_private_ip("172.32.0.1"));
    }

    #[test]
    fn blocks_192_168_network() {
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("192.168.1.100:8080"));
        assert!(is_private_ip("192.168.255.255"));
    }

    // --- Link-local / AWS metadata ---

    #[test]
    fn blocks_link_local() {
        assert!(is_private_ip("169.254.0.1"));
        assert!(is_private_ip("169.254.169.254")); // AWS metadata endpoint
        assert!(is_private_ip("169.254.169.254:80"));
    }

    // --- 0.0.0.0/8 (current network) ---

    #[test]
    fn blocks_zero_network() {
        assert!(is_private_ip("0.1.2.3"));
        assert!(is_private_ip("0.255.255.255"));
    }

    // --- IPv6 private ranges ---

    #[test]
    fn blocks_ipv4_mapped_ipv6() {
        assert!(is_private_ip("::ffff:127.0.0.1"));
        assert!(is_private_ip("::ffff:10.0.0.1"));
        assert!(is_private_ip("::ffff:192.168.1.1"));
    }

    #[test]
    fn allows_ipv4_mapped_ipv6_public() {
        assert!(!is_private_ip("::ffff:8.8.8.8"));
    }

    #[test]
    fn blocks_ipv6_unique_local() {
        assert!(is_private_ip("fc00::1"));
        assert!(is_private_ip("fd12::1"));
        assert!(is_private_ip("fdab:cdef::1"));
    }

    #[test]
    fn blocks_ipv6_link_local() {
        assert!(is_private_ip("fe80::1"));
        assert!(is_private_ip("fe80::abcd:1234"));
    }

    #[test]
    fn blocks_bracketed_ipv6_with_port() {
        assert!(is_private_ip("[::1]:8080"));
        assert!(is_private_ip("[fc00::1]:443"));
        assert!(is_private_ip("[fe80::1]:80"));
    }

    // --- Public IPs should be allowed ---

    #[test]
    fn allows_public_ips() {
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        assert!(!is_private_ip("203.0.113.1"));
        assert!(!is_private_ip("93.184.216.34:443"));
    }

    #[test]
    fn allows_public_ipv6() {
        // 2001:db8::/32 is documentation range but not in our private blocklist.
        assert!(!is_private_ip("2001:db8::1"));
    }

    #[test]
    fn allows_public_hostnames() {
        assert!(!is_private_ip("example.com"));
        assert!(!is_private_ip("example.com:443"));
        assert!(!is_private_ip("api.github.com"));
    }
}
