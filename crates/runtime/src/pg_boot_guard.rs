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

/// Distinct from the scheduler-leadership key (leader.rs) — that one
/// is HELD for the process lifetime by the winner; this one must be
/// briefly held by EVERY booting machine in turn.
const BOOT_DDL_LOCK_KEY: i64 = 0x70796c6f_6e5f6464; // "pylon_dd"

static GUARD: OnceLock<Mutex<Option<postgres::Client>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<postgres::Client>> {
    GUARD.get_or_init(|| Mutex::new(None))
}

/// Take the cluster-wide boot-DDL lock (blocking until the peer
/// currently booting finishes). Idempotent within the process: a
/// second call while held is a no-op, so every boot layer can call it
/// defensively. Connection failures are returned — the caller decides
/// whether boot can proceed (it can't: no connection here means no
/// connection for the DDL either).
pub fn acquire(database_url: &str) -> Result<(), String> {
    let mut held = slot().lock().unwrap_or_else(|p| p.into_inner());
    if held.is_some() {
        return Ok(());
    }
    let mut client = pylon_storage::postgres::live::connect_pg(database_url)
        .map_err(|e| format!("boot-ddl guard connect: {e}"))?;
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&BOOT_DDL_LOCK_KEY])
        .map_err(|e| format!("boot-ddl guard lock: {e}"))?;
    tracing::debug!("[boot] holding cluster boot-DDL lock");
    *held = Some(client);
    Ok(())
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
