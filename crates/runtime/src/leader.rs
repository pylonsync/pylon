//! Cross-machine leadership for singleton duties (the cron scheduler).
//!
//! N machines sharing one Postgres each run their own scheduler loop;
//! without coordination every cron task fires N times per match — dupe
//! emails, dupe rollups, dupe cleanups. Leadership makes the tick a
//! single-fire: only the machine holding the lease enqueues scheduled
//! jobs. Manually-enqueued and `runAfter`-style jobs are NOT gated —
//! they fire where the mutation ran, exactly once by construction.
//!
//! Two implementations:
//!
//! - [`AlwaysLeader`] — SQLite mode. SQLite is single-machine by
//!   definition (one writer, one file), so the process is trivially
//!   the leader. Zero overhead.
//! - [`PgAdvisoryLeader`] — Postgres mode. A dedicated connection
//!   (its own session — advisory locks are session-scoped, so it must
//!   NOT ride the pooled entity connections) takes
//!   `pg_try_advisory_lock(PYLON_SCHEDULER_LOCK_KEY)`. Holding the
//!   lock = being the leader. The background thread re-verifies the
//!   session every RECHECK_SECS; a dropped connection releases the
//!   lock server-side, the flag flips to false immediately, and the
//!   thread reconnects + re-contends — some OTHER machine may have won
//!   in between, which is the point.
//!
//! Failure semantics favor NOT firing: a machine that can't prove it
//! holds the lock acts as a follower. Worst case a cron tick is
//! skipped for one recheck interval during failover; the alternative
//! (fire when unsure) is the dupe storm this module exists to prevent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Advisory lock key for the scheduler singleton. Advisory locks are
/// scoped per-database, so distinct Pylon apps on separate databases
/// never contend; two apps SHARING one database would — give them
/// separate databases (already the documented deployment shape).
const SCHEDULER_LOCK_KEY: i64 = 0x70796c6f_6e5f7363; // "pylon_sc"

const RECHECK_SECS: u64 = 10;

pub trait Leadership: Send + Sync {
    /// Is THIS process currently allowed to run singleton duties?
    fn is_leader(&self) -> bool;
    /// Human tag for logs.
    fn describe(&self) -> &'static str;
}

/// Single-machine mode: always the leader.
pub struct AlwaysLeader;

impl Leadership for AlwaysLeader {
    fn is_leader(&self) -> bool {
        true
    }
    fn describe(&self) -> &'static str {
        "single-machine (always leader)"
    }
}

/// Postgres advisory-lock leadership. See module docs.
pub struct PgAdvisoryLeader {
    is_leader: Arc<AtomicBool>,
}

impl PgAdvisoryLeader {
    /// Spawn the leadership thread against `database_url`. Returns
    /// immediately; the flag flips true once (if) the lock is won.
    pub fn spawn(database_url: &str) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let thread_flag = Arc::clone(&flag);
        let url = database_url.to_string();
        std::thread::Builder::new()
            .name("pylon-scheduler-leader".into())
            .spawn(move || leadership_loop(&url, &thread_flag))
            .expect("spawn leadership thread");
        Self { is_leader: flag }
    }
}

impl Leadership for PgAdvisoryLeader {
    fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::Relaxed)
    }
    fn describe(&self) -> &'static str {
        "postgres advisory lock"
    }
}

fn leadership_loop(url: &str, flag: &AtomicBool) {
    let mut client: Option<postgres::Client> = None;
    let mut holding = false;
    loop {
        // (Re)connect on a DEDICATED session. Advisory locks live and
        // die with the session, so this connection does nothing else.
        if client.is_none() {
            match pylon_storage::postgres::live::connect_pg(url) {
                Ok(c) => client = Some(c),
                Err(e) => {
                    if holding || flag.load(Ordering::Relaxed) {
                        tracing::warn!(
                            "[scheduler] leadership connection lost and reconnect failed ({e}) — \
                             acting as follower until it heals"
                        );
                    }
                    holding = false;
                    flag.store(false, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_secs(RECHECK_SECS));
                    continue;
                }
            }
        }
        let c = client.as_mut().expect("client just ensured");
        let result: Result<bool, postgres::Error> = if holding {
            // Already hold the lock — the session owning it is proof.
            // A cheap liveness probe detects a silently-dead session.
            c.query_one("SELECT true", &[]).map(|_| true)
        } else {
            c.query_one("SELECT pg_try_advisory_lock($1)", &[&SCHEDULER_LOCK_KEY])
                .map(|row| row.get::<_, bool>(0))
        };
        match result {
            Ok(now_holding) => {
                if now_holding && !holding {
                    tracing::info!(
                        "[scheduler] acquired cluster leadership — this machine runs cron"
                    );
                }
                holding = now_holding;
                flag.store(now_holding, Ordering::Relaxed);
            }
            Err(e) => {
                // Session died: the server released our lock with it.
                // Demote FIRST, then reconnect and re-contend.
                if holding {
                    tracing::warn!(
                        "[scheduler] leadership session lost ({e}) — demoting; will re-contend"
                    );
                }
                holding = false;
                flag.store(false, Ordering::Relaxed);
                client = None;
            }
        }
        std::thread::sleep(Duration::from_secs(RECHECK_SECS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_leader_is_leader() {
        assert!(AlwaysLeader.is_leader());
    }

    /// Real two-session contention against Postgres. Env-gated like the
    /// pool tests: set PYLON_TEST_PG_URL to run.
    #[test]
    fn pg_advisory_lock_is_exclusive_across_sessions() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let a = PgAdvisoryLeader::spawn(&url);
        // Wait for A to win.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !a.is_leader() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(a.is_leader(), "first contender should acquire the lock");

        // B contends while A holds — must stay follower.
        let b = PgAdvisoryLeader::spawn(&url);
        std::thread::sleep(Duration::from_secs(2));
        assert!(!b.is_leader(), "second contender must NOT also lead");
        assert!(a.is_leader(), "holder keeps the lock");
    }
}
