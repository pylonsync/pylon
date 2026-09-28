use std::io::Result as IoResult;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::connection::Connection;
#[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
use crate::ssl::SslStream;

pub(crate) enum Stream {
    Http(Connection),
    #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
    Https(SslStream),
}

impl Clone for Stream {
    fn clone(&self) -> Self {
        match self {
            Stream::Http(tcp_stream) => Stream::Http(tcp_stream.try_clone().unwrap()),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => Stream::Https(ssl_stream.clone()),
        }
    }
}

impl From<Connection> for Stream {
    fn from(tcp_stream: Connection) -> Self {
        Stream::Http(tcp_stream)
    }
}

impl Stream {
    fn secure(&self) -> bool {
        match self {
            Stream::Http(_) => false,
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(_) => true,
        }
    }

    fn peer_addr(&mut self) -> IoResult<Option<SocketAddr>> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.peer_addr(),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.peer_addr(),
        }
    }

    fn shutdown(&mut self, how: Shutdown) -> IoResult<()> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.shutdown(how),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.shutdown(how),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.read(buf),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.write(buf),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.write(buf),
        }
    }

    fn flush(&mut self) -> IoResult<()> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.flush(),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.flush(),
        }
    }
}

pub struct RefinedTcpStream {
    stream: Stream,
    close_read: bool,
    close_write: bool,
    /// Pylon patch: shared by both halves. Set when a request hands the
    /// socket to other code ([`crate::Request::upgrade_detached`]): from
    /// then on tiny_http reads EOF from it and does not shut it down on
    /// drop, so the new owner keeps a working socket.
    detached: Arc<AtomicBool>,
}

/// Pylon patch: what a request needs to take over a plain-TCP connection.
///
/// Holds its own handle to the socket (a dup made once per connection),
/// so the socket stays open while any request of the connection exists.
/// A raw fd number was not enough: tiny_http ends an HTTP/1.0 (or
/// `Connection: close` / upgrade) connection as soon as it has read the
/// request, closing its fds while the request is still being handled; the
/// number could then be reused by the next accepted connection, and a
/// detach would take over that other client's socket.
#[derive(Clone)]
pub(crate) struct DetachHandle {
    socket: Arc<std::net::TcpStream>,
    pub(crate) flag: Arc<AtomicBool>,
}

impl DetachHandle {
    /// A new, independent handle to the same socket (dup / WSADuplicateSocket).
    pub(crate) fn clone_socket(&self) -> IoResult<std::net::TcpStream> {
        self.socket.try_clone()
    }

    pub(crate) fn set_detached(&self) {
        self.flag.store(true, Ordering::Release);
    }
}

impl RefinedTcpStream {
    pub(crate) fn new<S>(stream: S) -> (RefinedTcpStream, RefinedTcpStream)
    where
        S: Into<Stream>,
    {
        let stream: Stream = stream.into();

        let (read, write) = (stream.clone(), stream);
        let detached = Arc::new(AtomicBool::new(false));

        let read = RefinedTcpStream {
            stream: read,
            close_read: true,
            close_write: false,
            detached: Arc::clone(&detached),
        };

        let write = RefinedTcpStream {
            stream: write,
            close_read: false,
            close_write: true,
            detached,
        };

        (read, write)
    }

    /// Pylon patch: the handle a request uses to take over this connection.
    /// Plain TCP only; `None` for TLS and Unix sockets.
    pub(crate) fn detach_handle(&self) -> Option<DetachHandle> {
        match &self.stream {
            Stream::Http(crate::connection::Connection::Tcp(s)) => {
                let socket = s.try_clone().ok()?;
                Some(DetachHandle {
                    socket: Arc::new(socket),
                    flag: Arc::clone(&self.detached),
                })
            }
            _ => None,
        }
    }

    /// Returns true if this struct wraps around a secure connection.
    #[inline]
    pub(crate) fn secure(&self) -> bool {
        self.stream.secure()
    }

    pub(crate) fn peer_addr(&mut self) -> IoResult<Option<SocketAddr>> {
        self.stream.peer_addr()
    }
}

impl Drop for RefinedTcpStream {
    fn drop(&mut self) {
        if self.detached.load(Ordering::Acquire) {
            return;
        }
        if self.close_read {
            self.stream.shutdown(Shutdown::Read).ok();
        }

        if self.close_write {
            self.stream.shutdown(Shutdown::Write).ok();
        }
    }
}

impl Read for RefinedTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.detached.load(Ordering::Acquire) {
            return Ok(0);
        }
        self.stream.read(buf)
    }
}

impl Write for RefinedTcpStream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.stream.flush()
    }
}
