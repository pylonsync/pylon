//! Integration test for the cron single-fire lease.
//!
//! `register_app_crons` makes an app scaled to N replicas fire each cron
//! exactly once per tick by having every replica race to INSERT a
//! per-(function, minute) row into `_CronLease`; the unique `leaseKey`
//! lets exactly one win. These tests pin the invariants correctness
//! rests on:
//!
//! 1. `_CronLease` is auto-injected when the manifest declares any crons
//!    (and is server-only / unique so it never syncs and can dedupe).
//! 2. A second INSERT of the same `leaseKey` fails with a UNIQUE
//!    conflict — even from a different owner — the exact error
//!    `claim_cron_lease` keys off to decide Run vs Skip. Without this,
//!    multi-replica dedupe silently degrades to N-fires-per-tick.
//! 3. The winning row's `owner` reads back exactly as written, so the
//!    conflict path can tell "my own retry/restore" (Run) from "another
//!    replica won" (Skip).

use pylon_kernel::{AppManifest, ManifestCron};
use pylon_runtime::Runtime;

fn manifest_with_cron() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "cron-lease-e2e".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![ManifestCron {
            schedule: "*/5 * * * *".into(),
            function: "tick".into(),
            description: None,
        }],
        fonts: vec![],
    }
}

#[test]
fn cron_lease_entity_auto_injected() {
    let rt = Runtime::in_memory(manifest_with_cron()).unwrap();
    let ent = rt
        .manifest()
        .entities
        .iter()
        .find(|e| e.name == "_CronLease")
        .expect("framework must auto-inject _CronLease when crons are declared");

    let key = ent
        .fields
        .iter()
        .find(|f| f.name == "leaseKey")
        .expect("_CronLease has a leaseKey field");
    assert!(key.unique, "leaseKey must be unique — that's the dedupe");
    assert!(
        key.server_only,
        "leaseKey must be server-only so the lease never syncs to clients"
    );
    // No policy registered for `_CronLease` → default-denied to clients.
    assert!(
        !rt.manifest()
            .policies
            .iter()
            .any(|p| p.entity.as_deref() == Some("_CronLease")),
        "_CronLease must have NO policy (default-deny keeps it server-side)"
    );
}

#[test]
fn cron_lease_not_injected_without_crons() {
    let mut m = manifest_with_cron();
    m.crons.clear();
    let rt = Runtime::in_memory(m).unwrap();
    assert!(
        !rt.manifest()
            .entities
            .iter()
            .any(|e| e.name == "_CronLease"),
        "no crons declared → no _CronLease entity"
    );
}

#[test]
fn duplicate_lease_key_conflicts_regardless_of_owner() {
    // This is the atomic primitive `claim_cron_lease` depends on: the FIRST
    // insert of a (function, minute) leaseKey wins, and ANY second insert of
    // the same leaseKey fails with a UNIQUE conflict — even from a different
    // owner (the uniqueness is on leaseKey alone, not (leaseKey, owner)). The
    // conflict is what drives the owner read-back + Run/Skip decision.
    let rt = Runtime::in_memory(manifest_with_cron()).unwrap();

    rt.insert(
        "_CronLease",
        &serde_json::json!({ "leaseKey": "tick:28000000", "at": 1_680_000_000_i64, "owner": "replica-a" }),
    )
    .expect("first lease claim for a minute must succeed");

    let err = rt
        .insert(
            "_CronLease",
            &serde_json::json!({ "leaseKey": "tick:28000000", "at": 1_680_000_000_i64, "owner": "replica-b" }),
        )
        .expect_err("a different replica claiming the same leaseKey must fail");
    let msg = format!("{}: {}", err.code, err.message);
    assert!(
        msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key"),
        "duplicate leaseKey must surface a UNIQUE conflict (claim_cron_lease keys off this), got: {msg}"
    );

    // The conflict path reads the row back to compare owners — so the owner
    // the winner wrote must be exactly what a query returns.
    let rows = rt
        .query_filtered(
            "_CronLease",
            &serde_json::json!({ "leaseKey": "tick:28000000" }),
        )
        .expect("query the held lease");
    assert_eq!(
        rows.first()
            .and_then(|r| r.get("owner"))
            .and_then(|v| v.as_str()),
        Some("replica-a"),
        "the original owner must be readable so the conflict path can decide Run vs Skip"
    );

    // A DIFFERENT minute is a different key and must still be claimable —
    // otherwise the cron would fire once and then never again.
    rt.insert(
        "_CronLease",
        &serde_json::json!({ "leaseKey": "tick:28000001", "at": 1_680_000_060_i64, "owner": "replica-a" }),
    )
    .expect("a new minute's leaseKey is independent and must succeed");
}
