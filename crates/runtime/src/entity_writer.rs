//! Entity writes from inside the server (shards' buffered writes), through
//! the same pipeline as `PATCH /api/entities/<entity>/<id>`: plugin hooks,
//! the store, the change log, and sync broadcasts, with admin rights.

use std::sync::Arc;

use pylon_http::DataStore;
use pylon_policy::PolicyEngine;
use pylon_router::mutate::{self, MutationCtx, MutationError, MutationOp};
use pylon_router::ChangeNotifier;
use pylon_sync::ChangeLog;

use crate::datastore::PluginHooksAdapter;
use crate::Runtime;

pub struct EntityWriter {
    runtime: Arc<Runtime>,
    change_log: Arc<ChangeLog>,
    notifier: Arc<dyn ChangeNotifier>,
    policy: Arc<PolicyEngine>,
    plugins: PluginHooksAdapter,
}

/// Who may make a shard's buffered write: the shard, held by `machine`
/// under lease `epoch` (see the shard directory).
#[derive(Debug, Clone, Copy)]
pub struct Fence<'a> {
    pub shard: &'a str,
    pub machine: &'a str,
    pub epoch: i64,
}

/// Why a write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The row does not exist, or a plugin or policy refused the write.
    /// Writing it again gives the same answer.
    Refused(String),
    /// The store failed. Writing it again can succeed.
    Store(String),
}

impl From<pylon_http::DataError> for WriteError {
    /// A store error inside a fenced write: one the database may take on a
    /// later try is kept; one it refused for what it is (a constraint, a
    /// hook, validation) is not. The same rule as a shard call's retry.
    fn from(e: pylon_http::DataError) -> Self {
        let transient = crate::shard_wasm::retryable_code(&e.code);
        let why = format!("{}: {}", e.code, e.message);
        if transient {
            WriteError::Store(why)
        } else {
            WriteError::Refused(why)
        }
    }
}

impl EntityWriter {
    pub fn new(
        runtime: Arc<Runtime>,
        change_log: Arc<ChangeLog>,
        notifier: Arc<dyn ChangeNotifier>,
        policy: Arc<PolicyEngine>,
        plugins: Arc<pylon_plugin::PluginRegistry>,
    ) -> Self {
        Self {
            runtime,
            change_log,
            notifier,
            policy,
            plugins: PluginHooksAdapter(plugins),
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Set `fields` (a JSON object) on row `id` of `entity`, when `fence`
    /// still holds: on Postgres the check and the write are one
    /// transaction, with the shard's placement row share-locked, so a
    /// machine that lost the shard (its lease lapsed, or another machine
    /// took it) never writes after the one that has it. With no fence (no
    /// shard directory), or on SQLite (one machine), it is [`Self::update`].
    pub fn update_fenced(
        &self,
        entity: &str,
        id: &str,
        fields: &serde_json::Value,
        fence: Option<Fence<'_>>,
    ) -> Result<(), WriteError> {
        match (fence, self.runtime.pg_backend()) {
            (Some(fence), Some(pg)) => self.update_in_fence(pg, entity, id, fields, fence),
            _ => self.update(entity, id, fields),
        }
    }

    fn update_in_fence(
        &self,
        pg: &crate::PgBackend,
        entity: &str,
        id: &str,
        fields: &serde_json::Value,
        fence: Fence<'_>,
    ) -> Result<(), WriteError> {
        if self
            .runtime
            .manifest()
            .entities
            .iter()
            .all(|e| e.name != entity)
        {
            return Err(WriteError::Refused(format!("no entity \"{entity}\"")));
        }
        let crdt_hook: Arc<dyn pylon_storage::pg_tx_store::PgCrdtHook> =
            Arc::new(crate::pg_loro_store::PgCrdtHookImpl {
                crdt: Arc::clone(&pg.crdt),
                manifest: self.runtime.manifest_arc(),
            });
        let plugins = Arc::clone(&self.plugins.0);
        let pending = pg
            .store
            .with_transaction_crdt(crdt_hook, |inner: &dyn DataStore| {
                if !inner.check_shard_fence(fence.shard, fence.machine, fence.epoch)? {
                    return Err(WriteError::Refused(format!(
                        "shard {} is no longer held here under lease {}",
                        fence.shard, fence.epoch
                    )));
                }
                // Plugins run as on the entity API; change events are held
                // until the commit.
                let buffered = crate::datastore::PgBufferedTxStore::new(inner);
                let hooked = crate::datastore::HookEnforcingDataStore::new(
                    &buffered,
                    plugins,
                    pylon_auth::AuthContext::admin(),
                );
                if !hooked.update(entity, id, fields)? {
                    return Err(WriteError::Refused("no such row".into()));
                }
                drop(hooked);
                Ok(buffered.take_pending())
            })?;
        for ev in pending {
            let stored = self.change_log.append_with_prev(
                &ev.entity,
                &ev.row_id,
                ev.kind.clone(),
                ev.data.clone(),
                ev.prev_data.clone(),
            );
            pylon_router::broadcast_change_with_crdt(
                self.notifier.as_ref(),
                self.runtime.as_ref(),
                stored.seq,
                &ev.entity,
                &ev.row_id,
                ev.kind.clone(),
                ev.data.as_ref(),
                ev.prev_data.as_ref(),
            );
        }
        Ok(())
    }

    /// Set `fields` (a JSON object) on row `id` of `entity`.
    pub fn update(
        &self,
        entity: &str,
        id: &str,
        fields: &serde_json::Value,
    ) -> Result<(), WriteError> {
        if self
            .runtime
            .manifest()
            .entities
            .iter()
            .all(|e| e.name != entity)
        {
            return Err(WriteError::Refused(format!("no entity \"{entity}\"")));
        }
        let auth = pylon_auth::AuthContext::admin();
        let ctx = MutationCtx {
            store: self.runtime.as_ref(),
            change_log: &self.change_log,
            notifier: self.notifier.as_ref(),
            policy: &self.policy,
            plugin_hooks: &self.plugins,
            auth: &auth,
            bypass_policy: true,
        };
        match mutate::apply_mutation(
            &ctx,
            MutationOp::Update {
                entity,
                row_id: id,
                data: fields,
            },
        ) {
            Ok(_) => Ok(()),
            Err(MutationError::NotFound) => Err(WriteError::Refused("no such row".into())),
            Err(MutationError::PolicyDenied { reason, .. }) => Err(WriteError::Refused(reason)),
            Err(MutationError::Hook { code, message, .. }) => {
                Err(WriteError::Refused(format!("{code}: {message}")))
            }
            Err(MutationError::Store { code, message }) => {
                Err(WriteError::Store(format!("{code}: {message}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard_cluster::{PgShardDirectory, Placement};

    /// A shard's write lands while the shard is held under the fence's
    /// epoch, and is refused once another machine holds it.
    #[test]
    fn a_fenced_write_is_refused_once_the_shard_moved() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let manifest: pylon_kernel::AppManifest = serde_json::from_str(include_str!(
            "../../../examples/shard-arena/pylon.manifest.json"
        ))
        .unwrap();
        // Postgres takes its schema from the storage adapter, as `pylon
        // migrate` does.
        let mut adapter =
            pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
        let plan = adapter.plan_from_live(&manifest).unwrap();
        adapter.apply_plan(&plan).unwrap();
        let rt = Arc::new(Runtime::open_postgres(&url, manifest).unwrap());
        let dir = PgShardDirectory::open(rt.pg_backend().unwrap().store.shared_pool()).unwrap();
        let run = pylon_cluster::new_instance_id();
        let (shard, a, b) = (
            format!("fence-{run}"),
            format!("a-{run}"),
            format!("b-{run}"),
        );
        let placement = Placement {
            shard_id: shard.clone(),
            kind: format!("zone-{run}"),
            params: serde_json::json!({}),
            machine_id: a.clone(),
            pinned: false,
            failed: None,
            epoch: 11,
        };
        assert_eq!(
            dir.claim(&placement, 10).unwrap(),
            crate::shard_cluster::Claim::Claimed
        );
        // Gone when the test ends (a panic too): an orphan placement could
        // be taken over by a later test's machine.
        struct Release(Arc<pylon_storage::pg_datastore::PgPool>, String);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = self.0.with_client(|c| {
                    c.execute(
                        "DELETE FROM _pylon_shard_placements WHERE shard_id = $1",
                        &[&self.1],
                    )
                });
            }
        }
        let pool = rt.pg_backend().unwrap().store.shared_pool();
        let _release = Release(Arc::clone(&pool), shard.clone());
        let id = rt
            .insert(
                "Character",
                &serde_json::json!({ "userId": format!("u-{run}"), "x": 0, "nextGrant": 0 }),
            )
            .unwrap();
        let writer = EntityWriter::new(
            Arc::clone(&rt),
            Arc::new(ChangeLog::new()),
            Arc::new(pylon_router::NoopNotifier),
            Arc::new(PolicyEngine::from_manifest(rt.manifest())),
            Arc::new(pylon_plugin::PluginRegistry::new(rt.manifest().clone())),
        );
        fn fence<'a>(shard: &'a str, machine: &'a str, epoch: i64) -> Fence<'a> {
            Fence {
                shard,
                machine,
                epoch,
            }
        }
        let x = |n: i64| serde_json::json!({ "x": n });
        writer
            .update_fenced("Character", &id, &x(5), Some(fence(&shard, &a, 11)))
            .unwrap();
        assert_eq!(rt.get_by_id("Character", &id).unwrap().unwrap()["x"], 5);
        // Another epoch of the same machine: refused.
        assert!(matches!(
            writer.update_fenced("Character", &id, &x(6), Some(fence(&shard, &a, 12))),
            Err(WriteError::Refused(_))
        ));
        // Machine b takes it: a's write is refused, b's lands.
        assert!(dir.hand_over(&shard, &a, 11, &b, 21).unwrap());
        assert!(matches!(
            writer.update_fenced("Character", &id, &x(7), Some(fence(&shard, &a, 11))),
            Err(WriteError::Refused(_))
        ));
        assert_eq!(rt.get_by_id("Character", &id).unwrap().unwrap()["x"], 5);
        writer
            .update_fenced("Character", &id, &x(8), Some(fence(&shard, &b, 21)))
            .unwrap();
        assert_eq!(rt.get_by_id("Character", &id).unwrap().unwrap()["x"], 8);

        // A takeover waits for a fenced write in progress: b holds the
        // fence in an open transaction; a's hand-over back does not land
        // until that transaction ends.
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (end_tx, end_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let (rt, shard, b) = (Arc::clone(&rt), shard.clone(), b.clone());
            std::thread::spawn(move || {
                let manifest = rt.manifest().clone();
                rt.pg_backend()
                    .unwrap()
                    .store
                    .with_client(|c| {
                        let tx = c
                            .transaction()
                            .map_err(pylon_storage::pg_tx_store::pg_err_to_data)?;
                        let store = pylon_storage::pg_tx_store::PgTxStore::new(tx, &manifest);
                        assert!(store.check_shard_fence(&shard, &b, 21)?);
                        held_tx.send(()).unwrap();
                        end_rx.recv().unwrap();
                        store
                            .commit()
                            .map_err(pylon_storage::pg_tx_store::pg_err_to_data)?;
                        Ok::<(), pylon_http::DataError>(())
                    })
                    .unwrap();
            })
        };
        held_rx.recv().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let taker = {
            let (dir, shard, a, b) = (
                PgShardDirectory::open(Arc::clone(&pool)).unwrap(),
                shard.clone(),
                a.clone(),
                b.clone(),
            );
            std::thread::spawn(move || {
                done_tx
                    .send(dir.hand_over(&shard, &b, 21, &a, 31).unwrap())
                    .unwrap();
            })
        };
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(400))
                .is_err(),
            "the takeover did not wait for the fenced write"
        );
        end_tx.send(()).unwrap();
        assert!(done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap());
        holder.join().unwrap();
        taker.join().unwrap();
    }

    #[test]
    fn store_errors_are_kept_and_refusals_are_not() {
        let err = |code: &str| {
            WriteError::from(pylon_http::DataError {
                code: code.into(),
                message: "m".into(),
            })
        };
        for code in [
            "PG_TX_QUERY_FAILED",
            "PG_POOL_TIMEOUT",
            "SQLITE_BUSY",
            "TX_BEGIN_FAILED",
        ] {
            assert!(matches!(err(code), WriteError::Store(_)), "{code}");
        }
        for code in [
            "PG_REJECTED",
            "PG_INVALID_UPDATE",
            "PG_INVALID_DATA",
            "HOOK_REFUSED",
            "UPDATE_FAILED",
        ] {
            assert!(matches!(err(code), WriteError::Refused(_)), "{code}");
        }
    }
}
