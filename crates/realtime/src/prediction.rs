//! Client-side prediction and server reconciliation.
//!
//! For FPS-style games, the client simulates its own inputs immediately for
//! smooth responsiveness, while the server remains authoritative. When the
//! server processes an input, it sends back an [`InputAck`] with the server's
//! state at that tick. The client compares its predicted state to the
//! server's state and rolls back + replays if they diverge.
//!
//! This module defines the ack protocol primitives. The actual prediction
//! logic lives on the client; the server just needs to stamp snapshots with
//! the last-processed input sequence number per subscriber.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// InputAck
// ---------------------------------------------------------------------------

/// Acknowledges that the server has processed a client input up to a given
/// sequence number. Attach to each snapshot broadcast to that subscriber.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAck {
    /// Last server-assigned sequence number the server has processed for
    /// this subscriber.
    pub server_seq: u64,
    /// Last client-assigned sequence number, if the client tagged it.
    /// Lets the client match its own prediction buffer to what's been
    /// reconciled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_seq: Option<u64>,
    /// The shard tick when this input was applied.
    pub tick: u64,
}

// ---------------------------------------------------------------------------
// Reconciliation envelope
// ---------------------------------------------------------------------------

/// A broadcast envelope pairing a snapshot with per-subscriber ack info.
///
/// Games that need prediction wrap their `SimState::Snapshot` type in this
/// envelope, filled in during `snapshot_for(subscriber_id)` based on what
/// the server last applied for that subscriber.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation<T> {
    pub tick: u64,
    pub ack: Option<InputAck>,
    pub state: T,
}

impl<T> Reconciliation<T> {
    pub fn new(tick: u64, state: T) -> Self {
        Self {
            tick,
            ack: None,
            state,
        }
    }

    pub fn with_ack(mut self, ack: InputAck) -> Self {
        self.ack = Some(ack);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_serializes() {
        let r = Reconciliation::new(42, 3u32).with_ack(InputAck {
            server_seq: 100,
            client_seq: Some(7),
            tick: 42,
        });
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"tick\":42"));
        assert!(s.contains("\"server_seq\":100"));
    }

    #[test]
    fn reconciliation_without_ack_omits_field() {
        // ack is `Option<InputAck>`, but we explicitly serialize it. Keep the
        // field present as None so the client always sees it in the shape.
        let r: Reconciliation<u32> = Reconciliation::new(1, 0);
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"ack\":null"));
    }
}
