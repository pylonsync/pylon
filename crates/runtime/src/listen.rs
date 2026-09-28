//! Which network interfaces the servers listen on.
//!
//! `PYLON_HOST` picks the scope for every port the runtime opens (HTTP,
//! WebSocket, SSE, shard WebSocket):
//!
//! - unset, `0.0.0.0`, `::`, `*`, `all` → every interface, dual-stack
//!   (see [`crate::bind_dual_stack_tcp`]). This is the default for
//!   `pylon start` and the container image.
//! - `localhost`, `loopback`, `127.0.0.1`, `::1` → `127.0.0.1` and `::1`
//!   only. `pylon dev` sets this unless `--host` says otherwise.
//! - any other IP address → that address only.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{mpsc, Mutex};

/// Interfaces a server binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenScope {
    /// `127.0.0.1` and `::1`. Nothing outside this machine can connect.
    Loopback,
    /// Every interface.
    All,
    /// One address.
    Addr(IpAddr),
}

impl ListenScope {
    /// Parse a `PYLON_HOST` / `pylon dev --host` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        let v = value.trim();
        match v.to_ascii_lowercase().as_str() {
            "" | "0.0.0.0" | "::" | "[::]" | "*" | "all" => return Ok(Self::All),
            "localhost" | "loopback" | "127.0.0.1" | "::1" | "[::1]" => return Ok(Self::Loopback),
            _ => {}
        }
        let bare = v.trim_start_matches('[').trim_end_matches(']');
        bare.parse::<IpAddr>().map(Self::Addr).map_err(|_| {
            format!(
                "PYLON_HOST={value:?} is not a listen address. Use localhost (this machine only), \
                 0.0.0.0 (every interface), or one IP address such as 192.168.1.20."
            )
        })
    }

    /// Scope from `PYLON_HOST`. Unset means every interface.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("PYLON_HOST") {
            Ok(v) => Self::parse(&v),
            Err(_) => Ok(Self::All),
        }
    }

    /// True when only this machine can connect.
    pub fn is_loopback_only(&self) -> bool {
        match self {
            Self::Loopback => true,
            Self::All => false,
            Self::Addr(ip) => ip.is_loopback(),
        }
    }

    /// Host to print in "listening on" lines.
    pub fn display_host(&self) -> String {
        match self {
            Self::Loopback => "localhost".into(),
            Self::All => "0.0.0.0".into(),
            Self::Addr(IpAddr::V6(ip)) => format!("[{ip}]"),
            Self::Addr(ip) => ip.to_string(),
        }
    }
}

/// Bind `port` in `scope`. Returns one listener per socket: two for
/// [`ListenScope::Loopback`] when the machine has IPv6, one otherwise.
///
/// For `Loopback` with `port == 0`, the IPv6 socket reuses the port the IPv4
/// socket was given, so both answer on the same number.
pub fn bind_listeners(port: u16, scope: &ListenScope) -> std::io::Result<Vec<TcpListener>> {
    match scope {
        ListenScope::All => Ok(vec![crate::bind_dual_stack_tcp(port)?]),
        ListenScope::Addr(ip) => Ok(vec![TcpListener::bind(SocketAddr::new(*ip, port))?]),
        ListenScope::Loopback => {
            let v4 = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))?;
            let port = v4.local_addr()?.port();
            match TcpListener::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)) {
                Ok(v6) => Ok(vec![v4, v6]),
                // No IPv6 on this machine (or no ::1): 127.0.0.1 alone.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
                    ) || e.raw_os_error() == Some(libc_eafnosupport()) =>
                {
                    Ok(vec![v4])
                }
                Err(e) => Err(e),
            }
        }
    }
}

/// Whether a server could bind `port`. Each address a server may bind is
/// probed on its own: dual-stack `[::]`, `0.0.0.0`, `127.0.0.1`, and `::1`.
/// The port is busy when any of them fails with "address in use"; an
/// address family the machine lacks is skipped.
///
/// One probe is not enough. On macOS and BSD a `127.0.0.1` bind succeeds
/// while another socket holds `[::]:port`, and connections to
/// `127.0.0.1:port` then reach that socket. On Windows a `0.0.0.0` bind
/// succeeds in the same situation, so the dual-stack bind (which falls back
/// to `0.0.0.0`) cannot stand in for the `[::]` probe.
pub fn port_is_free(port: u16) -> bool {
    let probes: [fn(u16) -> std::io::Result<TcpListener>; 4] = [
        bind_v6_any_dual_stack,
        |p| TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), p)),
        |p| TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p)),
        |p| TcpListener::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), p)),
    ];
    probes.iter().all(|probe| match probe(port) {
        // Dropped at once, before the next probe binds.
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::AddrInUse,
    })
}

/// `[::]:port` accepting IPv4-mapped connections, with no IPv4 fallback.
fn bind_v6_any_dual_stack(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_only_v6(false)?;
    let addr: SocketAddr = (Ipv6Addr::UNSPECIFIED, port).into();
    socket.bind(&addr.into())?;
    socket.listen(1)?;
    Ok(socket.into())
}

#[cfg(unix)]
fn libc_eafnosupport() -> i32 {
    libc::EAFNOSUPPORT
}

#[cfg(not(unix))]
fn libc_eafnosupport() -> i32 {
    // WSAEAFNOSUPPORT
    10047
}

type Accepted = std::io::Result<(TcpStream, Option<IpAddr>)>;

/// Accepts connections from one or more listeners through one call.
///
/// With one listener, `accept` calls it directly. With several, one thread
/// per listener accepts and hands connections over a channel.
pub struct Listeners {
    addrs: Vec<SocketAddr>,
    inner: Inner,
}

enum Inner {
    One(TcpListener),
    Many(Mutex<mpsc::Receiver<Accepted>>),
}

impl Listeners {
    /// Bind `port` in `scope`.
    pub fn bind(port: u16, scope: &ListenScope) -> std::io::Result<Self> {
        Self::from_vec(bind_listeners(port, scope)?)
    }

    /// Wrap already-bound listeners.
    pub fn from_vec(mut listeners: Vec<TcpListener>) -> std::io::Result<Self> {
        let addrs = listeners
            .iter()
            .map(TcpListener::local_addr)
            .collect::<std::io::Result<Vec<_>>>()?;
        if listeners.len() == 1 {
            return Ok(Self {
                addrs,
                inner: Inner::One(listeners.remove(0)),
            });
        }
        let (tx, rx) = mpsc::sync_channel::<Accepted>(0);
        for listener in listeners {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("pylon-accept".into())
                .spawn(move || loop {
                    let res = crate::accept_tcp(&listener);
                    let failed = res.is_err();
                    if tx.send(res).is_err() {
                        return;
                    }
                    if failed {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                })?;
        }
        Ok(Self {
            addrs,
            inner: Inner::Many(Mutex::new(rx)),
        })
    }

    /// Addresses the listeners are bound to.
    pub fn local_addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    /// Block until a connection arrives on any listener. Same panic-proof
    /// accept as [`crate::accept_tcp`].
    pub fn accept(&self) -> Accepted {
        match &self.inner {
            Inner::One(l) => crate::accept_tcp(l),
            Inner::Many(rx) => {
                let rx = rx.lock().unwrap_or_else(|p| p.into_inner());
                rx.recv().unwrap_or_else(|_| {
                    Err(std::io::Error::other("every accept thread has exited"))
                })
            }
        }
    }
}

impl From<TcpListener> for Listeners {
    fn from(l: TcpListener) -> Self {
        let addrs = l.local_addr().map(|a| vec![a]).unwrap_or_default();
        Self {
            addrs,
            inner: Inner::One(l),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_port_held_dual_stack_is_not_free() {
        let held = crate::bind_dual_stack_tcp(0).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!super::port_is_free(port));
        drop(held);
        assert!(super::port_is_free(port));
    }

    #[test]
    fn a_port_held_on_loopback_is_not_free() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!super::port_is_free(port));
    }

    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn parse_accepts_the_documented_values() {
        for v in [
            "localhost",
            "LOCALHOST",
            "loopback",
            "127.0.0.1",
            "::1",
            "[::1]",
        ] {
            assert_eq!(ListenScope::parse(v).unwrap(), ListenScope::Loopback, "{v}");
        }
        for v in ["0.0.0.0", "::", "[::]", "*", "all", ""] {
            assert_eq!(ListenScope::parse(v).unwrap(), ListenScope::All, "{v}");
        }
        assert_eq!(
            ListenScope::parse("192.168.1.20").unwrap(),
            ListenScope::Addr("192.168.1.20".parse().unwrap())
        );
        assert_eq!(
            ListenScope::parse("[fe80::1]").unwrap(),
            ListenScope::Addr("fe80::1".parse().unwrap())
        );
        assert!(ListenScope::parse("my-laptop.local").is_err());
    }

    #[test]
    fn loopback_scope_binds_only_loopback_addresses() {
        let l = Listeners::bind(0, &ListenScope::Loopback).expect("bind loopback");
        assert!(!l.local_addrs().is_empty());
        for addr in l.local_addrs() {
            assert!(addr.ip().is_loopback(), "{addr} is not loopback");
        }
        // Both sockets share one port.
        let port = l.local_addrs()[0].port();
        assert!(l.local_addrs().iter().all(|a| a.port() == port));
        assert!(ListenScope::Loopback.is_loopback_only());
        assert!(!ListenScope::All.is_loopback_only());
    }

    #[test]
    fn loopback_scope_accepts_on_every_loopback_family() {
        let l = std::sync::Arc::new(Listeners::bind(0, &ListenScope::Loopback).unwrap());
        let addrs: Vec<SocketAddr> = l.local_addrs().to_vec();
        let server = std::sync::Arc::clone(&l);
        let n = addrs.len();
        std::thread::spawn(move || {
            for _ in 0..n {
                let (mut s, peer) = server.accept().expect("accept");
                assert!(peer.is_some_and(|p| p.is_loopback()));
                let _ = s.write_all(b"ok");
            }
        });
        for addr in addrs {
            let mut c = TcpStream::connect(addr).expect("connect");
            let mut buf = String::new();
            c.read_to_string(&mut buf).unwrap();
            assert_eq!(buf, "ok", "{addr}");
        }
    }

    #[test]
    fn all_scope_binds_the_unspecified_address() {
        let l = Listeners::bind(0, &ListenScope::All).unwrap();
        assert!(l.local_addrs()[0].ip().is_unspecified());
    }
}
