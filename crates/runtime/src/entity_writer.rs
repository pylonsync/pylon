//! Entity writes from inside the server (shards' buffered writes), through
//! the same pipeline as `PATCH /api/entities/<entity>/<id>`: plugin hooks,
//! the store, the change log, and sync broadcasts, with admin rights.

use std::sync::Arc;

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

/// Why a write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The row does not exist, or a plugin or policy refused the write.
    /// Writing it again gives the same answer.
    Refused(String),
    /// The store failed. Writing it again can succeed.
    Store(String),
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
