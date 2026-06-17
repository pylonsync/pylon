//! SSRF egress guard — classify an IP as unsafe for a server-initiated
//! outbound request, and resolve a host while dropping unsafe addresses.
//!
//! Lives in `pylon-kernel` (pure `std::net`, no deps) so every crate that
//! fetches a URL whose host is even indirectly user-influenced shares ONE
//! vetted classification predicate — the place SSRF bugs hide. The
//! ureq-specific wiring (no-follow agent, per-hop allowlist re-check)
//! stays in each consumer crate; only the predicate + resolver are shared.
//!
//! Consumers: the image optimizer (`/_pylon/image`), OIDC issuer discovery
//! (org SSO), and (planned) outbound webhooks.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

/// Returns `true` if `ip` must NOT be the target of a server-initiated
/// outbound request. Blocks loopback, private (RFC1918), link-local
/// (incl. the 169.254.169.254 cloud-metadata range), CGNAT shared space,
/// "this-network" 0.0.0.0/8, broadcast, documentation, and multicast —
/// plus the IPv6 equivalents (loopback, unique-local, link-local,
/// unspecified, multicast). IPv4-mapped / -compatible IPv6 addresses are
/// re-checked as their embedded IPv4 (so `::ffff:169.254.169.254` is
/// blocked).
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                // Covers ::ffff:a.b.c.d (mapped) and ::a.b.c.d (compatible),
                // including ::1 -> 0.0.0.1 and :: -> 0.0.0.0, all of which
                // the v4 classifier also rejects.
                return is_blocked_ipv4(&v4);
            }
            is_blocked_ipv6(v6)
        }
    }
}

fn is_blocked_ipv4(ip: &Ipv4Addr) -> bool {
    ip.is_loopback()            // 127.0.0.0/8
        || ip.is_private()      // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()   // 169.254.0.0/16 (incl. cloud metadata)
        || ip.is_broadcast()    // 255.255.255.255
        || ip.is_documentation()// 192.0.2/24, 198.51.100/24, 203.0.113/24
        || ip.is_unspecified()  // 0.0.0.0
        || ip.is_multicast()    // 224.0.0.0/4
        || is_shared_v4(ip)     // 100.64.0.0/10 (CGNAT, RFC6598)
        || ip.octets()[0] == 0 // 0.0.0.0/8 "this network" (RFC791)
}

/// 100.64.0.0/10 — `Ipv4Addr::is_shared` is unstable, so check by hand.
fn is_shared_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

fn is_blocked_ipv6(ip: &Ipv6Addr) -> bool {
    ip.is_loopback()                  // ::1
        || ip.is_unspecified()        // ::
        || ip.is_multicast()          // ff00::/8
        || (ip.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        || (ip.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local unicast
}

/// Resolve `netloc` ("host:port") to socket addresses, dropping any that
/// [`is_blocked_ip`] rejects. Errors if no safe address remains.
///
/// Designed to be plugged in as a ureq custom resolver: ureq connects to
/// exactly the addresses this returns, so the resolution that validates is
/// the one used to connect — closing the DNS-rebinding TOCTOU gap (no
/// second, unvalidated lookup at connect time). A direct private-IP target
/// (`http://169.254.169.254/...`) is rejected here too.
pub fn resolve_guarded(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    let safe: Vec<SocketAddr> = netloc
        .to_socket_addrs()?
        .filter(|a| !is_blocked_ip(&a.ip()))
        .collect();
    if safe.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to connect to a private/loopback/link-local address (SSRF guard)",
        ));
    }
    Ok(safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(s: &str) -> bool {
        is_blocked_ip(&s.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn blocks_ipv4_internal_ranges() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            assert!(blocked(ip), "{ip} must be blocked");
        }
    }

    #[test]
    fn allows_public_ipv4() {
        for ip in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "93.184.216.34"] {
            assert!(!blocked(ip), "{ip} must be allowed");
        }
    }

    #[test]
    fn blocks_ipv6_internal_and_mapped() {
        for ip in [
            "::1",                    // loopback
            "::",                     // unspecified
            "fe80::1",                // link-local
            "fc00::1",                // unique-local
            "fd12:3456::1",           // unique-local
            "ff02::1",                // multicast
            "::ffff:127.0.0.1",       // mapped loopback
            "::ffff:169.254.169.254", // mapped metadata
            "::ffff:10.0.0.1",        // mapped private
        ] {
            assert!(blocked(ip), "{ip} must be blocked");
        }
    }

    #[test]
    fn allows_public_ipv6() {
        for ip in [
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(!blocked(ip), "{ip} must be allowed");
        }
    }

    #[test]
    fn resolve_guarded_rejects_loopback_literal() {
        // 127.0.0.1 resolves to itself and is blocked → error, no addrs.
        let err = resolve_guarded("127.0.0.1:80").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_guarded_allows_public_literal() {
        let addrs = resolve_guarded("8.8.8.8:443").expect("public IP must resolve");
        assert!(!addrs.is_empty());
        assert!(addrs.iter().all(|a| !is_blocked_ip(&a.ip())));
    }
}
