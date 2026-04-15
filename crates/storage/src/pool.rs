use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

/// A minimal database connection pool.
///
/// Maintains a bounded set of connections and hands them out on request.
/// When all connections are in use, callers block until one is returned
/// (or a timeout expires).
///
/// Connections are returned automatically when the [`PooledConnection`] guard
/// is dropped, so callers cannot accidentally leak a slot.
pub struct ConnectionPool<T> {
    inner: Mutex<VecDeque<T>>,
    available: Condvar,
    max_size: usize,
}

/// RAII guard that returns the connection to the pool on drop.
pub struct PooledConnection<'a, T> {
    pool: &'a ConnectionPool<T>,
    conn: Option<T>,
}

impl<T> std::fmt::Debug for PooledConnection<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConnection")
            .field("has_conn", &self.conn.is_some())
            .finish()
    }
}

impl<T> ConnectionPool<T> {
    /// Create a new, empty pool with the given capacity.
    ///
    /// Connections must be added via [`add`] before they can be acquired.
    pub fn new(max_size: usize) -> Self {
        assert!(max_size > 0, "pool max_size must be at least 1");
        Self {
            inner: Mutex::new(VecDeque::with_capacity(max_size)),
            available: Condvar::new(),
            max_size,
        }
    }

    /// Add a connection to the pool.
    ///
    /// Panics if the pool is already at capacity.
    pub fn add(&self, conn: T) {
        let mut queue = self.inner.lock().expect("pool lock poisoned");
        assert!(
            queue.len() < self.max_size,
            "cannot add connection: pool is at capacity ({})",
            self.max_size,
        );
        queue.push_back(conn);
        self.available.notify_one();
    }

    /// Acquire a connection, blocking up to `timeout`.
    ///
    /// Returns `Err` if the timeout expires before a connection becomes
    /// available.
    pub fn get(&self, timeout: Duration) -> Result<PooledConnection<'_, T>, PoolError> {
        let mut queue = self.inner.lock().expect("pool lock poisoned");

        // Fast path: a connection is already available.
        if let Some(conn) = queue.pop_front() {
            return Ok(PooledConnection {
                pool: self,
                conn: Some(conn),
            });
        }

        // Slow path: wait for a connection to be returned.
        let (mut queue, wait_result) = self
            .available
            .wait_timeout_while(queue, timeout, |q| q.is_empty())
            .expect("pool lock poisoned");

        if wait_result.timed_out() && queue.is_empty() {
            return Err(PoolError::Timeout);
        }

        match queue.pop_front() {
            Some(conn) => Ok(PooledConnection {
                pool: self,
                conn: Some(conn),
            }),
            None => Err(PoolError::Unavailable),
        }
    }

    /// Number of connections currently idle in the pool.
    pub fn available_count(&self) -> usize {
        self.inner.lock().expect("pool lock poisoned").len()
    }

    /// Maximum number of connections this pool can hold.
    pub fn max_size(&self) -> usize {
        self.max_size
    }
}

// ---------------------------------------------------------------------------
// Pool errors
// ---------------------------------------------------------------------------

/// Error returned when a connection cannot be acquired from the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// The timeout expired before a connection became available.
    Timeout,
    /// No connection was available after waiting (spurious wakeup).
    Unavailable,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::Timeout => write!(f, "connection pool: timed out waiting for a connection"),
            PoolError::Unavailable => {
                write!(f, "connection pool: no connection available after wait")
            }
        }
    }
}

impl std::error::Error for PoolError {}

// ---------------------------------------------------------------------------
// PooledConnection — RAII guard
// ---------------------------------------------------------------------------

impl<T> Drop for PooledConnection<'_, T> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.add(conn);
        }
    }
}

impl<T> std::ops::Deref for PooledConnection<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.conn
            .as_ref()
            .expect("PooledConnection used after take (bug)")
    }
}

impl<T> std::ops::DerefMut for PooledConnection<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.conn
            .as_mut()
            .expect("PooledConnection used after take (bug)")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn basic_get_and_return() {
        let pool = ConnectionPool::new(2);
        pool.add("conn1");
        pool.add("conn2");

        assert_eq!(pool.available_count(), 2);
        assert_eq!(pool.max_size(), 2);

        {
            let c = pool.get(Duration::from_millis(100)).unwrap();
            assert_eq!(*c, "conn1");
            assert_eq!(pool.available_count(), 1);
        }

        // Guard dropped, connection returned.
        assert_eq!(pool.available_count(), 2);
    }

    #[test]
    fn pool_exhaustion_blocks_then_succeeds() {
        let pool = Arc::new(ConnectionPool::new(1));
        pool.add(42u32);

        // Spawn a thread that grabs the connection, holds it briefly, then
        // releases it.
        let pool2 = Arc::clone(&pool);
        let holder = thread::spawn(move || {
            let _conn = pool2.get(Duration::from_millis(100)).unwrap();
            assert_eq!(*_conn, 42);
            thread::sleep(Duration::from_millis(100));
            // _conn drops here, returning connection to pool.
        });

        // Give the holder thread time to acquire the connection.
        thread::sleep(Duration::from_millis(20));

        // This thread blocks until the holder releases.
        let c = pool.get(Duration::from_secs(2)).unwrap();
        assert_eq!(*c, 42);

        holder.join().expect("holder thread panicked");
    }

    #[test]
    fn pool_exhaustion_timeout() {
        let pool = ConnectionPool::new(1);
        pool.add("only");

        let _held = pool.get(Duration::from_millis(100)).unwrap();
        let result = pool.get(Duration::from_millis(50));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PoolError::Timeout);
    }

    #[test]
    fn dropped_guard_returns_connection() {
        let pool = ConnectionPool::new(1);
        pool.add(99u32);

        assert_eq!(pool.available_count(), 1);
        {
            let _c = pool.get(Duration::from_millis(100)).unwrap();
            assert_eq!(pool.available_count(), 0);
        }
        assert_eq!(pool.available_count(), 1);
    }

    #[test]
    fn multiple_concurrent_gets() {
        let pool = Arc::new(ConnectionPool::new(4));
        for i in 0..4u32 {
            pool.add(i);
        }

        let mut handles = Vec::new();
        for _ in 0..8 {
            let pool = Arc::clone(&pool);
            handles.push(thread::spawn(move || {
                let c = pool.get(Duration::from_secs(2)).unwrap();
                // Simulate work.
                thread::sleep(Duration::from_millis(10));
                let _val = *c;
            }));
        }

        for h in handles {
            h.join().expect("thread panicked");
        }

        assert_eq!(pool.available_count(), 4);
    }

    #[test]
    fn deref_mut_works() {
        let pool = ConnectionPool::new(1);
        pool.add(vec![1, 2, 3]);

        let mut c = pool.get(Duration::from_millis(100)).unwrap();
        c.push(4);
        assert_eq!(*c, vec![1, 2, 3, 4]);
    }

    #[test]
    #[should_panic(expected = "pool max_size must be at least 1")]
    fn zero_size_panics() {
        let _pool = ConnectionPool::<u32>::new(0);
    }

    #[test]
    #[should_panic(expected = "pool is at capacity")]
    fn add_beyond_capacity_panics() {
        let pool = ConnectionPool::new(1);
        pool.add(1);
        pool.add(2);
    }

    #[test]
    fn pool_error_display() {
        assert!(format!("{}", PoolError::Timeout).contains("timed out"));
        assert!(format!("{}", PoolError::Unavailable).contains("no connection"));
    }
}
