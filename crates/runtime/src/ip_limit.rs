//! Per-IP concurrent connection limiter used by every streaming endpoint
//! (WS notifier, SSE, shard WS). A single misbehaving peer should not be
//! able to exhaust the server's thread budget or per-client mutex pool by
//! opening hundreds of long-lived sockets.
//!
//! The limiter is cheap: one mutex, one HashMap entry per active IP. An
//! RAII guard released on disconnect decrements the count — callers cannot
//! leak a slot by forgetting to release it, even on panic.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Default cap on concurrent streaming connections per client IP. Generous
/// enough for normal browser tabs, chatty mobile apps, or shared NATs, but
/// stingy enough that one attacker can't open 10k sockets. Each endpoint
/// can override by constructing the counter with a different cap.
pub const DEFAULT_MAX_CONNECTIONS_PER_IP: u32 = 64;

/// Tracks how many concurrent streaming connections each IP currently holds.
pub struct IpConnCounter {
    counts: Mutex<HashMap<IpAddr, u32>>,
    cap: u32,
}

impl Default for IpConnCounter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONNECTIONS_PER_IP)
    }
}

impl IpConnCounter {
    pub fn new(cap: u32) -> Self {
        Self {
            counts: Mutex::new(HashMap::new()),
            cap,
        }
    }

    /// Increment the counter for `ip` if it hasn't hit the cap. Returns a
    /// guard that decrements on drop.
    pub fn acquire(self: &Arc<Self>, ip: IpAddr) -> Option<IpConnGuard> {
        let mut map = self.counts.lock().unwrap();
        let slot = map.entry(ip).or_insert(0);
        if *slot >= self.cap {
            return None;
        }
        *slot += 1;
        Some(IpConnGuard {
            counter: Arc::clone(self),
            ip,
        })
    }

    #[cfg(test)]
    pub(crate) fn get(&self, ip: IpAddr) -> u32 {
        self.counts.lock().unwrap().get(&ip).copied().unwrap_or(0)
    }
}

/// RAII guard: decrements the IP's connection count when dropped. Hold it
/// for the full lifetime of the connection (thread, task) so the slot is
/// only released on actual disconnect.
pub struct IpConnGuard {
    counter: Arc<IpConnCounter>,
    ip: IpAddr,
}

impl Drop for IpConnGuard {
    fn drop(&mut self) {
        let mut map = self.counter.counts.lock().unwrap();
        if let Some(count) = map.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_cap() {
        let counter = Arc::new(IpConnCounter::new(3));
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let g1 = counter.acquire(ip).unwrap();
        let _g2 = counter.acquire(ip).unwrap();
        let _g3 = counter.acquire(ip).unwrap();
        assert!(counter.acquire(ip).is_none(), "at cap, next acquire fails");
        drop(g1);
        assert!(counter.acquire(ip).is_some(), "freed slot is reusable");
    }

    #[test]
    fn frees_on_drop() {
        let counter = Arc::new(IpConnCounter::new(3));
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        {
            let _g = counter.acquire(ip).unwrap();
            assert_eq!(counter.get(ip), 1);
        }
        assert_eq!(counter.get(ip), 0, "empty entries evicted");
    }

    #[test]
    fn isolates_ips() {
        let counter = Arc::new(IpConnCounter::new(2));
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        let b: IpAddr = "192.0.2.2".parse().unwrap();
        let _a1 = counter.acquire(a).unwrap();
        let _a2 = counter.acquire(a).unwrap();
        assert!(counter.acquire(a).is_none());
        assert!(counter.acquire(b).is_some(), "other IP not starved");
    }
}
