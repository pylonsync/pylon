//! Who gets dev mode's owner-level shortcuts.
//!
//! Dev mode (`PYLON_DEV_MODE=1`, set by `pylon dev`) turns on shortcuts that
//! give full control of the app to whoever calls them:
//!
//! - `POST /api/auth/session` and `/api/auth/upgrade` mint a session for any
//!   user id.
//! - Magic-code, email-verification, phone, and magic-link responses carry
//!   the code (`dev_code`, `dev_token`).
//! - The OAuth callback accepts a caller-supplied email.
//! - `/admin/*` and `/metrics` are open when no operator token is set, which
//!   includes minting a Studio ticket.
//! - `/_pylon/dev/files/*` writes files into the workspace, and
//!   `/_pylon/dev/render` renders workspace modules.
//!
//! These are for the developer at the keyboard. A request gets them only
//! when it comes from this machine: the TCP peer is a loopback address and
//! the `Host` header names a loopback host. The `Host` check stops DNS
//! rebinding (a page on `evil.example` re-pointed at 127.0.0.1 connects from
//! loopback but sends `Host: evil.example`) and tunnels such as ngrok, which
//! connect from loopback on behalf of the internet.
//!
//! `PYLON_DEV_TRUST_REMOTE=1` gives the shortcuts to every caller. Use it
//! only on a network you control.

use std::net::IpAddr;

/// True when `PYLON_DEV_MODE` is `1` or `true` (any case). Every other value,
/// including `false` and `0`, is production.
pub fn dev_mode_enabled() -> bool {
    env_flag("PYLON_DEV_MODE")
}

/// `PYLON_DEV_TRUST_REMOTE`: give dev shortcuts to callers on other machines.
pub fn trust_remote_enabled() -> bool {
    env_flag("PYLON_DEV_TRUST_REMOTE")
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

/// Loopback, including an IPv4 loopback peer seen through a dual-stack
/// socket as `::ffff:127.0.0.1`.
pub fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// True when a `Host` header value names this machine: `localhost`,
/// `*.localhost`, `127.x.x.x`, or `[::1]`, with or without a port. Also
/// `10.0.2.2` and `10.0.3.2`, the addresses the Android emulator and
/// Genymotion use for the host machine's loopback: their connections arrive
/// from 127.0.0.1 with that `Host`.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        // [v6]:port
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
            _ => host,
        }
    };
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return true;
    }
    if name == "10.0.2.2" || name == "10.0.3.2" {
        return true;
    }
    name.parse::<IpAddr>().is_ok_and(is_loopback_ip)
}

/// Whether a request from `peer` carrying `host` gets dev shortcuts.
///
/// `peer` must be the TCP peer address, never a forwarded-for header. A
/// missing `Host` header is accepted: browsers always send one, so only a
/// local non-browser client omits it.
pub fn shortcuts_allowed(
    is_dev: bool,
    trust_remote: bool,
    peer: Option<IpAddr>,
    host: Option<&str>,
) -> bool {
    if !is_dev {
        return false;
    }
    if trust_remote {
        return true;
    }
    peer.is_some_and(is_loopback_ip) && host.is_none_or(is_loopback_host)
}

/// [`shortcuts_allowed`] for a live request, reading dev mode and
/// `PYLON_DEV_TRUST_REMOTE` from the environment.
pub fn request_shortcuts_allowed(request: &tiny_http::Request) -> bool {
    let host = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str());
    shortcuts_allowed(
        dev_mode_enabled(),
        trust_remote_enabled(),
        request.remote_addr().map(|a| a.ip()),
        host,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn local_caller_gets_shortcuts_in_dev_only() {
        assert!(shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("localhost:4321")
        ));
        assert!(shortcuts_allowed(
            true,
            false,
            ip("::1"),
            Some("[::1]:4321")
        ));
        assert!(shortcuts_allowed(
            true,
            false,
            ip("::ffff:127.0.0.1"),
            Some("127.0.0.1:4321")
        ));
        assert!(shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("app.localhost:4321")
        ));
        assert!(shortcuts_allowed(true, false, ip("127.0.0.1"), None));
        // Production never gets them, whoever calls.
        assert!(!shortcuts_allowed(
            false,
            false,
            ip("127.0.0.1"),
            Some("localhost")
        ));
        assert!(!shortcuts_allowed(
            false,
            true,
            ip("127.0.0.1"),
            Some("localhost")
        ));
    }

    #[test]
    fn network_callers_do_not_get_shortcuts() {
        for peer in [
            "192.168.1.40",
            "10.0.0.2",
            "172.17.0.1",
            "fe80::1",
            "::ffff:192.168.1.40",
        ] {
            assert!(
                !shortcuts_allowed(true, false, ip(peer), Some("192.168.1.10:4321")),
                "{peer}"
            );
        }
        // Unknown peer (truncated sockaddr) fails closed.
        assert!(!shortcuts_allowed(true, false, None, Some("localhost")));
    }

    #[test]
    fn rebinding_and_tunnels_do_not_get_shortcuts() {
        // Loopback peer, but the browser thinks it's talking to another site.
        assert!(!shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("evil.example:4321")
        ));
        assert!(!shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("abc.ngrok.io")
        ));
        assert!(!shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("localhost.evil.example")
        ));
        assert!(!shortcuts_allowed(
            true,
            false,
            ip("127.0.0.1"),
            Some("192.168.1.10:4321")
        ));
    }

    #[test]
    fn trust_remote_opts_every_caller_in() {
        assert!(shortcuts_allowed(
            true,
            true,
            ip("192.168.1.40"),
            Some("192.168.1.10:4321")
        ));
    }

    #[test]
    fn loopback_host_parsing() {
        for h in [
            "localhost",
            "LOCALHOST:80",
            "localhost.",
            "a.b.localhost:1",
            "127.0.0.1",
            "127.9.9.9:4321",
            "[::1]",
            "[::1]:4321",
            "10.0.2.2:4321",
        ] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in [
            "",
            "example.com",
            "0.0.0.0:4321",
            "[::]:4321",
            "10.0.0.1",
            "localhostx",
            "[::1",
        ] {
            assert!(!is_loopback_host(h), "{h}");
        }
    }
}
