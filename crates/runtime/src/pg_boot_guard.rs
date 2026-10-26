//! Cross-machine serialization of boot-time DDL.
//!
//! Every Postgres-backed subsystem bootstraps its own tables at boot
//! with `CREATE TABLE IF NOT EXISTS` — the entity schema, the CRDT
//! sidecar, sessions, oauth state, magic codes, api keys, audit, SAML,
//! the cron-lease table. IF NOT EXISTS is idempotent but NOT
//! race-free: two machines cold-booting one fresh database can both
//! pass the existence check and collide in Postgres's catalog
//! (duplicate `pg_type`/`pg_class` key), killing one boot with a
//! generic "db error". A rolling two-machine deploy hits exactly this.
//!
//! Rather than sprinkle a lock into ~15 backends, the whole boot
//! window is serialized: [`acquire`] opens ONE dedicated connection
//! and takes a session advisory lock before the first DDL runs;
//! [`release`] unlocks + drops it once the server is fully
//! initialized. The second machine's boot blocks in `acquire` until
//! the first finishes its DDL pass, then runs its own now-no-op pass.
//! Boot serialization costs seconds, once, on rollout — steady state
//! pays nothing.
//!
//! Process-global (OnceLock) because boot DDL spans layers that don't
//! share state: `Runtime::open_postgres` (entity store, sidecar,
//! cron-lease) acquires; `start_server` releases after the auth/state
//! backends have bootstrapped. Short-lived CLI paths (migrate, seed)
//! that never call `release` hold the lock only until process exit —
//! the session dies with them and Postgres frees the lock.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Distinct from the scheduler-leadership key (leader.rs) — that one
/// is HELD for the process lifetime by the winner; this one must be
/// briefly held by EVERY booting machine in turn.
const BOOT_DDL_LOCK_KEY: i64 = 0x70796c6f_6e5f6464; // "pylon_dd"

static GUARD: OnceLock<Mutex<Option<postgres::Client>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<postgres::Client>> {
    GUARD.get_or_init(|| Mutex::new(None))
}

/// How long `acquire` waits for a peer's boot DDL when
/// `PYLON_BOOT_LOCK_TIMEOUT_SECS` is unset. A peer's boot DDL takes seconds,
/// so a longer wait means a stuck session holds the lock.
const DEFAULT_WAIT_SECS: u64 = 60;
/// How often `acquire` tries the lock again while a peer holds it.
const RETRY_EVERY: Duration = Duration::from_millis(500);
/// How often `acquire` logs who holds the lock while it waits.
const LOG_EVERY: Duration = Duration::from_secs(10);

/// Take the cluster-wide boot-DDL lock, waiting while the peer currently
/// booting finishes. Idempotent within the process: a second call while
/// held is a no-op, so every boot layer can call it defensively.
///
/// The wait has a limit. While it waits it logs the session that holds the
/// lock; after the limit it returns an error and boot fails, so the process
/// exits and the platform restarts it, instead of a server that never
/// listens. Connection failures are returned too — no connection here means
/// no connection for the DDL either.
pub fn acquire(database_url: &str) -> Result<(), String> {
    let mut held = slot().lock().unwrap_or_else(|p| p.into_inner());
    if held.is_some() {
        return Ok(());
    }
    let mut client = pylon_storage::postgres::live::connect_pg(database_url)
        .map_err(|e| format!("boot-ddl guard connect: {e}"))?;
    wait_for_lock(&mut client, BOOT_DDL_LOCK_KEY, wait_limit_from_env())?;
    tracing::debug!("[boot] holding cluster boot-DDL lock");
    *held = Some(client);
    Ok(())
}

/// Take the advisory lock `key` on `client`, trying again every
/// [`RETRY_EVERY`] and logging the holder every [`LOG_EVERY`]. Fails after
/// `limit`.
fn wait_for_lock(client: &mut postgres::Client, key: i64, limit: Duration) -> Result<(), String> {
    let started = Instant::now();
    let mut last_log: Option<Instant> = None;
    loop {
        let got: bool = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
            .map_err(|e| format!("boot-ddl guard lock: {e}"))?
            .get(0);
        if got {
            return Ok(());
        }
        let waited = started.elapsed();
        if waited >= limit {
            return Err(format!(
                "boot-ddl guard lock: another session held the boot lock for {}s ({}). \
                 End that session, or raise PYLON_BOOT_LOCK_TIMEOUT_SECS",
                waited.as_secs(),
                describe_holder(client, key)
            ));
        }
        if last_log.map_or(true, |t| t.elapsed() >= LOG_EVERY) {
            tracing::warn!(
                "[boot] waiting for the boot lock ({}s of {}s): held by {}",
                waited.as_secs(),
                limit.as_secs(),
                describe_holder(client, key)
            );
            last_log = Some(Instant::now());
        }
        std::thread::sleep(RETRY_EVERY);
    }
}

/// The session holding advisory lock `key`, for the log: pid, client
/// address, application name, state and age. Postgres splits a bigint
/// advisory key into `classid` (high 32 bits) and `objid` (low 32 bits).
fn describe_holder(client: &mut postgres::Client, key: i64) -> String {
    let high = key >> 32;
    let low = key & 0xffff_ffff;
    let row = client.query_opt(
        "SELECT a.pid, coalesce(host(a.client_addr), 'unknown'), \
                coalesce(nullif(a.application_name, ''), 'unknown'), \
                coalesce(a.state, 'unknown'), \
                coalesce(extract(epoch FROM now() - a.backend_start)::bigint, 0) \
         FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
         WHERE l.locktype = 'advisory' AND l.granted \
           AND l.classid::bigint = $1 AND l.objid::bigint = $2 AND l.objsubid = 1 \
         LIMIT 1",
        &[&high, &low],
    );
    match row {
        Ok(Some(r)) => format!(
            "pid {} from {} ({}, {}, connected {}s ago)",
            r.get::<_, i32>(0),
            r.get::<_, String>(1),
            r.get::<_, String>(2),
            r.get::<_, String>(3),
            r.get::<_, i64>(4)
        ),
        Ok(None) => "a session this role cannot see".into(),
        Err(e) => format!("unknown session ({e})"),
    }
}

fn wait_limit_from_env() -> Duration {
    parse_wait_limit(
        std::env::var("PYLON_BOOT_LOCK_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Zero or an unparseable value falls back to the default: zero would fail
/// every boot that meets a peer mid-DDL.
fn parse_wait_limit(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_WAIT_SECS);
    Duration::from_secs(secs)
}

/// Release the boot-DDL lock once init is complete. Safe to call when
/// not held (no-op).
pub fn release() {
    let mut held = slot().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(mut client) = held.take() {
        let _ = client.query_one("SELECT pg_advisory_unlock($1)", &[&BOOT_DDL_LOCK_KEY]);
        tracing::debug!("[boot] released cluster boot-DDL lock");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Two processes' worth of contention simulated with raw clients:
    /// while the guard is held, a second session's try-lock fails;
    /// after release it succeeds. Env-gated: PYLON_TEST_PG_URL.
    #[test]
    fn wait_limit_defaults_and_parses() {
        assert_eq!(
            parse_wait_limit(None),
            Duration::from_secs(DEFAULT_WAIT_SECS)
        );
        assert_eq!(
            parse_wait_limit(Some("0")),
            Duration::from_secs(DEFAULT_WAIT_SECS)
        );
        assert_eq!(
            parse_wait_limit(Some("x")),
            Duration::from_secs(DEFAULT_WAIT_SECS)
        );
        assert_eq!(parse_wait_limit(Some(" 5 ")), Duration::from_secs(5));
    }

    /// A session that holds the boot lock and never lets go makes the
    /// wait fail after its limit, naming the holder, instead of
    /// blocking forever. Env-gated: PYLON_TEST_PG_URL.
    #[test]
    fn wait_gives_up_when_a_session_keeps_the_lock() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        // Its own key: Postgres runtimes opened by other tests in this
        // process hold the real boot lock.
        const TEST_KEY: i64 = 0x70796c6f_6e5f7474;
        let mut holder = pylon_storage::postgres::live::connect_pg(&url).expect("holder session");
        let locked: bool = holder
            .query_one("SELECT pg_try_advisory_lock($1)", &[&TEST_KEY])
            .unwrap()
            .get(0);
        assert!(locked, "the holder session must get the lock");
        let holder_pid: i32 = holder
            .query_one("SELECT pg_backend_pid()", &[])
            .unwrap()
            .get(0);

        let mut waiter = pylon_storage::postgres::live::connect_pg(&url).expect("waiter session");
        let started = Instant::now();
        let err = wait_for_lock(&mut waiter, TEST_KEY, Duration::from_secs(2))
            .expect_err("must time out");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waited too long"
        );
        assert!(
            err.contains(&format!("pid {holder_pid}")),
            "error names the holder: {err}"
        );

        let _ = holder.query_one("SELECT pg_advisory_unlock($1)", &[&TEST_KEY]);
    }

    #[test]
    fn guard_excludes_second_session_until_release() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        acquire(&url).expect("acquire");
        let mut other = pylon_storage::postgres::live::connect_pg(&url).expect("second session");
        let contended: bool = other
            .query_one("SELECT pg_try_advisory_lock($1)", &[&BOOT_DDL_LOCK_KEY])
            .unwrap()
            .get(0);
        assert!(
            !contended,
            "second session must NOT get the lock while held"
        );

        release();
        // The other session should acquire promptly now.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = false;
        while Instant::now() < deadline {
            got = other
                .query_one("SELECT pg_try_advisory_lock($1)", &[&BOOT_DDL_LOCK_KEY])
                .unwrap()
                .get(0);
            if got {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(got, "lock must be free after release");
        let _ = other.query_one("SELECT pg_advisory_unlock($1)", &[&BOOT_DDL_LOCK_KEY]);
    }
}
