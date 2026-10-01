//! A WebTransport client with a C ABI, for engines whose runtime has no
//! QUIC: the Pylon C# client loads it as a native plugin (Unity's Mono and
//! IL2CPP have no WebTransport).
//!
//! The caller polls; nothing calls back across the boundary, so the
//! caller's threads and IL2CPP stay simple. A session is a handle (a
//! nonzero `u64`). [`pylon_wt_connect`] starts it in the background:
//!
//! 1. it connects to the URL, pinning the given SHA-256 certificate hashes
//!    (none: the server's certificate must chain to a public CA);
//! 2. it opens one bidirectional stream;
//! 3. [`pylon_wt_state`] then reports [`STATE_OPEN`].
//!
//! From then on the caller writes the stream with [`pylon_wt_stream_write`],
//! drains received stream bytes with [`pylon_wt_stream_read`] and received
//! datagrams with [`pylon_wt_recv_datagram`], and sends datagrams with
//! [`pylon_wt_send_datagram`]. [`pylon_wt_stream_queued`] gives the stream
//! bytes still waiting to go out (a peer that stops reading holds them;
//! past [`MAX_QUEUED`] writes fail). [`pylon_wt_free`] ends it and releases
//! the handle. The stream carries the caller's own framing; this crate does
//! not parse it.
//!
//! Every function catches panics and reports them as errors, and checks
//! its handle and pointers: a stale or unknown handle returns
//! [`ERR_UNKNOWN_HANDLE`].

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use wtransport::{ClientConfig, Connection, Endpoint, VarInt};

/// The session is connecting (and opening its stream).
pub const STATE_CONNECTING: i32 = 0;
/// The session and its stream are open.
pub const STATE_OPEN: i32 = 1;
/// The session closed: by [`pylon_wt_close`], or by the server.
pub const STATE_CLOSED: i32 = 2;
/// The session did not open, or failed; [`pylon_wt_error`] says why.
pub const STATE_FAILED: i32 = 3;

pub const ERR_UNKNOWN_HANDLE: i32 = -1;
/// Nothing is waiting (a receive), or the session is not open (a send).
pub const ERR_NONE: i32 = -2;
/// The caller's buffer is too small for the next datagram (it stays queued).
pub const ERR_BUFFER_TOO_SMALL: i32 = -3;
/// A bad argument (a null pointer, a bad URL, a hash list that is not whole hashes).
pub const ERR_ARGUMENT: i32 = -4;
/// The send failed (the datagram is larger than the session allows, or the session ended).
pub const ERR_SEND: i32 = -5;
/// A panic inside the plugin.
pub const ERR_PANIC: i32 = -6;
/// The stream ended and every received byte has been read.
pub const ERR_STREAM_ENDED: i32 = -7;
/// [`MAX_QUEUED`] bytes wait to be written to the stream.
pub const ERR_FULL: i32 = -8;

/// Stream bytes a session holds that the peer has not taken yet, at most.
/// A writer should keep its own lower limit with [`pylon_wt_stream_queued`].
pub const MAX_QUEUED: u64 = 64 * 1024 * 1024;

/// Received stream bytes kept unread at most; past this the session fails
/// (the caller stopped draining).
const MAX_STREAM_BUFFER: usize = 64 * 1024 * 1024;
/// Received datagrams kept unread at most; past this the oldest is dropped
/// (a datagram is lossy by nature).
const MAX_DATAGRAMS: usize = 4096;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pylon-wt")
            .enable_all()
            .build()
            .expect("start the WebTransport runtime")
    })
}

#[derive(Default)]
struct Received {
    stream: VecDeque<u8>,
    stream_ended: bool,
    datagrams: VecDeque<Vec<u8>>,
}

struct Session {
    state: AtomicI32,
    received: Mutex<Received>,
    error: Mutex<String>,
    /// The server's application close code and reason, when it closed the session.
    peer_close: Mutex<Option<(u32, Vec<u8>)>>,
    connection: Mutex<Option<Connection>>,
    outbound: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    /// Bytes given to `pylon_wt_stream_write` and not yet written to the stream.
    queued: Arc<AtomicU64>,
    /// Set to true to stop the session's tasks.
    stop: watch::Sender<bool>,
}

impl Session {
    fn fail(&self, message: String) {
        *self.error.lock().unwrap() = message;
        self.state.store(STATE_FAILED, Ordering::SeqCst);
    }
}

fn sessions() -> &'static Mutex<HashMap<u64, Arc<Session>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<u64, Arc<Session>>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session(handle: u64) -> Option<Arc<Session>> {
    sessions().lock().unwrap().get(&handle).cloned()
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// The server's keep-alive and idle timeout (crates/runtime/src/shard_wt.rs):
/// a quiet session stays open, and a dead one ends within 30 s.
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const MAX_IDLE: Duration = Duration::from_secs(30);
/// The handshake and the stream must open within this.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

fn client_config(hashes: Vec<[u8; 32]>) -> Result<ClientConfig, String> {
    let builder = ClientConfig::builder().with_bind_default();
    if !hashes.is_empty() {
        return builder
            .with_server_certificate_hashes(
                hashes.into_iter().map(wtransport::tls::Sha256Digest::new),
            )
            .keep_alive_interval(Some(KEEP_ALIVE))
            .max_idle_timeout(Some(MAX_IDLE))
            .map(|b| b.build())
            .map_err(|e| format!("idle timeout: {e}"));
    }
    // A CA-signed certificate: the public roots bundled here, so it works
    // the same on platforms without a native store this stack can read.
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| format!("TLS: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    builder
        .with_custom_tls(tls)
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(Some(MAX_IDLE))
        .map(|b| b.build())
        .map_err(|e| format!("idle timeout: {e}"))
}

async fn run(session: Arc<Session>, url: String, hashes: Vec<[u8; 32]>) {
    let mut stop = session.stop.subscribe();
    let opened = async {
        let config = client_config(hashes)?;
        let endpoint = Endpoint::client(config).map_err(|e| format!("endpoint: {e}"))?;
        let connection = endpoint
            .connect(url.as_str())
            .await
            .map_err(|e| format!("connect: {e}"))?;
        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("open the stream: {e}"))?
            .await
            .map_err(|e| format!("open the stream: {e}"))?;
        Ok::<_, String>((endpoint, connection, send, recv))
    };
    let (_endpoint, connection, mut send, mut recv) = tokio::select! {
        r = tokio::time::timeout(OPEN_TIMEOUT, opened) => match r {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return session.fail(e),
            Err(_) => return session.fail(format!("connect: no answer in {} s", OPEN_TIMEOUT.as_secs())),
        },
        _ = stop.changed() => return,
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    *session.connection.lock().unwrap() = Some(connection.clone());
    *session.outbound.lock().unwrap() = Some(tx);
    session.state.store(STATE_OPEN, Ordering::SeqCst);

    let queued = Arc::clone(&session.queued);
    let writer = async {
        while let Some(bytes) = rx.recv().await {
            if let Err(e) = send.write_all(&bytes).await {
                return Err(format!("write the stream: {e}"));
            }
            queued.fetch_sub(bytes.len() as u64, Ordering::SeqCst);
        }
        Ok(())
    };
    let reader = async {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match recv.read(&mut buf).await {
                Ok(Some(n)) => {
                    let mut r = session.received.lock().unwrap();
                    if r.stream.len() + n > MAX_STREAM_BUFFER {
                        return Err("more than 64 MiB of stream bytes waited unread".to_string());
                    }
                    r.stream.extend(&buf[..n]);
                }
                Ok(None) => {
                    session.received.lock().unwrap().stream_ended = true;
                    return Ok(());
                }
                Err(e) => {
                    session.received.lock().unwrap().stream_ended = true;
                    return Err(format!("read the stream: {e}"));
                }
            }
        }
    };
    let datagrams = async {
        loop {
            match connection.receive_datagram().await {
                Ok(d) => {
                    let mut r = session.received.lock().unwrap();
                    if r.datagrams.len() >= MAX_DATAGRAMS {
                        r.datagrams.pop_front();
                    }
                    r.datagrams.push_back(d.payload().to_vec());
                }
                Err(e) => return e,
            }
        }
    };
    let closed = connection.closed();

    let failure = tokio::select! {
        _ = stop.changed() => None,
        r = writer => r.err(),
        r = reader => r.err(),
        _ = datagrams => None,
        _ = closed => None,
    };
    // The stream fails as soon as the server closes the session: report the
    // server's close (its code and reason) rather than the stream error.
    let reason = tokio::time::timeout(std::time::Duration::from_millis(500), connection.closed())
        .await
        .ok();
    match reason {
        Some(e) => {
            if let wtransport::error::ConnectionError::ApplicationClosed(c) = &e {
                *session.peer_close.lock().unwrap() =
                    Some((c.code().into_inner() as u32, c.reason().to_vec()));
            }
            *session.error.lock().unwrap() = format!("the session ended: {e}");
            let _ = session.state.compare_exchange(
                STATE_OPEN,
                STATE_CLOSED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
        None => {
            if let Some(e) = failure {
                session.fail(e);
            }
        }
    }
    // A session that ended for another reason still reads as ended.
    let _ = session.state.compare_exchange(
        STATE_OPEN,
        STATE_CLOSED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    session.received.lock().unwrap().stream_ended = true;
    *session.outbound.lock().unwrap() = None;
}

/// Start a session to `url` (an `https://` WebTransport URL, NUL-terminated
/// UTF-8). `hashes` holds `hash_count` SHA-256 certificate hashes, 32
/// bytes each, back to back; with none, the certificate must chain to a
/// public CA. Returns the handle, or 0 on a bad argument.
///
/// # Safety
/// `url` must be a NUL-terminated string; `hashes` must point to
/// `hash_count * 32` readable bytes (or be null when `hash_count` is 0).
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_connect(
    url: *const c_char,
    hashes: *const u8,
    hash_count: usize,
) -> u64 {
    guard(0, || {
        if url.is_null() || (hash_count > 0 && hashes.is_null()) || hash_count > 64 {
            return 0;
        }
        let Ok(url) = CStr::from_ptr(url).to_str().map(str::to_string) else {
            return 0;
        };
        if !url.starts_with("https://") {
            return 0;
        }
        let bytes = if hash_count == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(hashes, hash_count * 32)
        };
        let hashes: Vec<[u8; 32]> = bytes.as_chunks::<32>().0.to_vec();

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        let (stop, _) = watch::channel(false);
        let s = Arc::new(Session {
            state: AtomicI32::new(STATE_CONNECTING),
            received: Mutex::new(Received::default()),
            error: Mutex::new(String::new()),
            peer_close: Mutex::new(None),
            connection: Mutex::new(None),
            outbound: Mutex::new(None),
            queued: Arc::new(AtomicU64::new(0)),
            stop,
        });
        sessions().lock().unwrap().insert(handle, Arc::clone(&s));
        runtime().spawn(run(s, url, hashes));
        handle
    })
}

/// The session's state: [`STATE_CONNECTING`], [`STATE_OPEN`],
/// [`STATE_CLOSED`], or [`STATE_FAILED`]; [`ERR_UNKNOWN_HANDLE`] for an
/// unknown handle.
#[no_mangle]
pub extern "C" fn pylon_wt_state(handle: u64) -> i32 {
    guard(ERR_PANIC, || match session(handle) {
        Some(s) => s.state.load(Ordering::SeqCst),
        None => ERR_UNKNOWN_HANDLE,
    })
}

/// The largest datagram the session can send now, in bytes (0 before it opens).
#[no_mangle]
pub extern "C" fn pylon_wt_max_datagram_size(handle: u64) -> i32 {
    guard(ERR_PANIC, || match session(handle) {
        Some(s) => s
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|c| c.max_datagram_size())
            .map(|n| n.min(i32::MAX as usize) as i32)
            .unwrap_or(0),
        None => ERR_UNKNOWN_HANDLE,
    })
}

/// Send one datagram. 0 on success.
///
/// # Safety
/// `data` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_send_datagram(handle: u64, data: *const u8, len: usize) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        if data.is_null() && len > 0 {
            return ERR_ARGUMENT;
        }
        let bytes = if len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(data, len)
        };
        let conn = s.connection.lock().unwrap().clone();
        match conn {
            Some(c) if s.state.load(Ordering::SeqCst) == STATE_OPEN => match c.send_datagram(bytes)
            {
                Ok(()) => 0,
                Err(_) => ERR_SEND,
            },
            _ => ERR_NONE,
        }
    })
}

/// Copy the oldest received datagram into `buf` and remove it. Returns its
/// length, [`ERR_NONE`] when none is waiting, or [`ERR_BUFFER_TOO_SMALL`]
/// (the datagram stays queued).
///
/// # Safety
/// `buf` must point to `cap` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_recv_datagram(handle: u64, buf: *mut u8, cap: usize) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        if buf.is_null() && cap > 0 {
            return ERR_ARGUMENT;
        }
        let mut r = s.received.lock().unwrap();
        let Some(front) = r.datagrams.front() else {
            return ERR_NONE;
        };
        if front.len() > cap {
            return ERR_BUFFER_TOO_SMALL;
        }
        let d = r.datagrams.pop_front().unwrap();
        if !d.is_empty() {
            std::ptr::copy_nonoverlapping(d.as_ptr(), buf, d.len());
        }
        d.len().min(i32::MAX as usize) as i32
    })
}

/// Queue bytes for the stream, in order. 0 on success, [`ERR_NONE`] when
/// the session is not open, [`ERR_FULL`] when they would take the queue past
/// [`MAX_QUEUED`] (the peer stopped reading).
///
/// # Safety
/// `data` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_stream_write(handle: u64, data: *const u8, len: usize) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        if data.is_null() && len > 0 {
            return ERR_ARGUMENT;
        }
        let bytes = if len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data, len).to_vec()
        };
        let Some(tx) = s.outbound.lock().unwrap().clone() else {
            return ERR_NONE;
        };
        let n = bytes.len() as u64;
        // Reserve the bytes first: concurrent writers cannot pass the limit together.
        if s.queued.fetch_add(n, Ordering::SeqCst) + n > MAX_QUEUED {
            s.queued.fetch_sub(n, Ordering::SeqCst);
            return ERR_FULL;
        }
        if tx.send(bytes).is_err() {
            s.queued.fetch_sub(n, Ordering::SeqCst);
            return ERR_NONE;
        }
        0
    })
}

/// Stream bytes queued by [`pylon_wt_stream_write`] and not yet written to
/// the stream (QUIC flow control holds them while the peer does not read),
/// or [`ERR_UNKNOWN_HANDLE`].
#[no_mangle]
pub extern "C" fn pylon_wt_stream_queued(handle: u64) -> i64 {
    guard(ERR_PANIC as i64, || match session(handle) {
        Some(s) => s.queued.load(Ordering::SeqCst).min(i64::MAX as u64) as i64,
        None => ERR_UNKNOWN_HANDLE as i64,
    })
}

/// Copy up to `cap` received stream bytes into `buf`. Returns how many (0
/// when none are waiting), or [`ERR_STREAM_ENDED`] once the stream ended and
/// everything was read.
///
/// # Safety
/// `buf` must point to `cap` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_stream_read(handle: u64, buf: *mut u8, cap: usize) -> i64 {
    guard(ERR_PANIC as i64, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE as i64;
        };
        if buf.is_null() && cap > 0 {
            return ERR_ARGUMENT as i64;
        }
        let mut r = s.received.lock().unwrap();
        if r.stream.is_empty() {
            return if r.stream_ended {
                ERR_STREAM_ENDED as i64
            } else {
                0
            };
        }
        let n = r.stream.len().min(cap);
        let (a, b) = r.stream.as_slices();
        let first = a.len().min(n);
        std::ptr::copy_nonoverlapping(a.as_ptr(), buf, first);
        if n > first {
            std::ptr::copy_nonoverlapping(b.as_ptr(), buf.add(first), n - first);
        }
        r.stream.drain(..n);
        n as i64
    })
}

/// Close the session with an application code and reason (UTF-8, `len`
/// bytes). The handle stays valid until [`pylon_wt_free`].
///
/// # Safety
/// `reason` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_close(
    handle: u64,
    code: u32,
    reason: *const u8,
    len: usize,
) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        let reason = if reason.is_null() || len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(reason, len)
        };
        if let Some(c) = s.connection.lock().unwrap().as_ref() {
            c.close(VarInt::from_u32(code), reason);
        }
        let _ = s.stop.send(true);
        let state = s.state.load(Ordering::SeqCst);
        if state == STATE_CONNECTING || state == STATE_OPEN {
            s.state.store(STATE_CLOSED, Ordering::SeqCst);
        }
        0
    })
}

/// When the server closed the session with an application code: writes
/// the code to `*code`, copies up to `cap` bytes of the reason into `buf`,
/// and returns the reason's full length. [`ERR_NONE`] when it did not.
///
/// # Safety
/// `code` must be writable; `buf` must point to `cap` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_close_info(
    handle: u64,
    code: *mut u32,
    buf: *mut u8,
    cap: usize,
) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        let Some((c, reason)) = s.peer_close.lock().unwrap().clone() else {
            return ERR_NONE;
        };
        if !code.is_null() {
            *code = c;
        }
        if !buf.is_null() {
            let n = reason.len().min(cap);
            std::ptr::copy_nonoverlapping(reason.as_ptr(), buf, n);
        }
        reason.len().min(i32::MAX as usize) as i32
    })
}

/// Copy up to `cap` bytes of the last error message (UTF-8) into `buf` and
/// return its full length (0 when there is none).
///
/// # Safety
/// `buf` must point to `cap` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pylon_wt_error(handle: u64, buf: *mut u8, cap: usize) -> i32 {
    guard(ERR_PANIC, || {
        let Some(s) = session(handle) else {
            return ERR_UNKNOWN_HANDLE;
        };
        let e = s.error.lock().unwrap().clone();
        if !buf.is_null() {
            let n = e.len().min(cap);
            std::ptr::copy_nonoverlapping(e.as_ptr(), buf, n);
        }
        e.len().min(i32::MAX as usize) as i32
    })
}

/// End the session (if it is still open) and release the handle.
#[no_mangle]
pub extern "C" fn pylon_wt_free(handle: u64) {
    guard((), || {
        if let Some(s) = sessions().lock().unwrap().remove(&handle) {
            if let Some(c) = s.connection.lock().unwrap().as_ref() {
                c.close(VarInt::from_u32(0), b"");
            }
            let _ = s.stop.send(true);
        }
    })
}

/// The plugin's version (the Pylon version it was built from), NUL-terminated.
#[no_mangle]
pub extern "C" fn pylon_wt_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}
