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
#[derive(Clone)]
pub(crate) struct DetachHandle {
    #[cfg(unix)]
    pub(crate) raw: std::os::unix::io::RawFd,
    #[cfg(windows)]
    pub(crate) raw: std::os::windows::io::RawSocket,
    pub(crate) flag: Arc<AtomicBool>,
}

impl DetachHandle {
    /// A new, independent handle to the same socket (dup / WSADuplicateSocket).
    #[allow(unsafe_code)]
    pub(crate) fn clone_socket(&self) -> IoResult<std::net::TcpStream> {
        #[cfg(unix)]
        {
            // SAFETY: the connection owns this fd and is alive while the
            // request that holds this handle exists.
            let fd = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(self.raw) };
            Ok(std::net::TcpStream::from(fd.try_clone_to_owned()?))
        }
        #[cfg(windows)]
        {
            // SAFETY: as above, for the socket handle.
            let s = unsafe { std::os::windows::io::BorrowedSocket::borrow_raw(self.raw) };
            Ok(std::net::TcpStream::from(s.try_clone_to_owned()?))
        }
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
                #[cfg(unix)]
                let raw = std::os::unix::io::AsRawFd::as_raw_fd(s);
                #[cfg(windows)]
                let raw = std::os::windows::io::AsRawSocket::as_raw_socket(s);
                Some(DetachHandle {
                    raw,
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
