use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Change events — the append-only log entries
// ---------------------------------------------------------------------------

/// A change event in the sync log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// The entity that was changed.
    pub entity: String,
    /// The row ID that was changed.
    pub row_id: String,
    /// The type of change.
    pub kind: ChangeKind,
    /// The data after the change (None for deletes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Timestamp of the change.
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Insert,
    Update,
    Delete,
}

// ---------------------------------------------------------------------------
// Sync cursor — tracks client position in the log
// ---------------------------------------------------------------------------

/// A sync cursor representing a client's position in the change log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor {
    /// The last sequence number the client has seen.
    pub last_seq: u64,
}

impl SyncCursor {
    pub fn beginning() -> Self {
        Self { last_seq: 0 }
    }
}

// ---------------------------------------------------------------------------
// Pull response — what the server sends to a pulling client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    /// Changes since the client's cursor.
    pub changes: Vec<ChangeEvent>,
    /// The new cursor position after these changes.
    pub cursor: SyncCursor,
    /// Whether there are more changes to pull.
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// Push request — what a client sends to push changes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    /// The changes the client wants to push.
    pub changes: Vec<ClientChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientChange {
    pub entity: String,
    pub row_id: String,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Change log — in-memory append-only log
// ---------------------------------------------------------------------------

/// An in-memory change log for development.
pub struct ChangeLog {
    events: Mutex<Vec<ChangeEvent>>,
    seq: Mutex<u64>,
}

impl ChangeLog {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            seq: Mutex::new(0),
        }
    }

    /// Append a change event. Returns the assigned sequence number.
    pub fn append(&self, entity: &str, row_id: &str, kind: ChangeKind, data: Option<serde_json::Value>) -> u64 {
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let event = ChangeEvent {
            seq: *seq,
            entity: entity.to_string(),
            row_id: row_id.to_string(),
            kind,
            data,
            timestamp: now_iso8601(),
        };
        self.events.lock().unwrap().push(event);
        *seq
    }

    /// Pull changes since a cursor, up to a limit.
    pub fn pull(&self, cursor: &SyncCursor, limit: usize) -> PullResponse {
        let events = self.events.lock().unwrap();
        let changes: Vec<ChangeEvent> = events
            .iter()
            .filter(|e| e.seq > cursor.last_seq)
            .take(limit)
            .cloned()
            .collect();

        let last_seq = changes.last().map(|e| e.seq).unwrap_or(cursor.last_seq);
        let has_more = events.iter().any(|e| e.seq > last_seq);

        PullResponse {
            changes,
            cursor: SyncCursor { last_seq },
            has_more,
        }
    }

    /// Get the total number of events in the log.
    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.lock().unwrap().is_empty()
    }
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}Z", ts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log() {
        let log = ChangeLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn append_and_pull() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, Some(serde_json::json!({"name": "Alice"})));
        log.append("User", "u2", ChangeKind::Insert, Some(serde_json::json!({"name": "Bob"})));

        assert_eq!(log.len(), 2);

        let resp = log.pull(&SyncCursor::beginning(), 100);
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.cursor.last_seq, 2);
        assert!(!resp.has_more);
    }

    #[test]
    fn pull_with_cursor() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        log.append("User", "u2", ChangeKind::Insert, None);
        log.append("User", "u3", ChangeKind::Insert, None);

        // Pull from seq 1 — should get events 2 and 3.
        let resp = log.pull(&SyncCursor { last_seq: 1 }, 100);
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.changes[0].seq, 2);
        assert_eq!(resp.changes[1].seq, 3);
    }

    #[test]
    fn pull_with_limit() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        log.append("User", "u2", ChangeKind::Insert, None);
        log.append("User", "u3", ChangeKind::Insert, None);

        let resp = log.pull(&SyncCursor::beginning(), 2);
        assert_eq!(resp.changes.len(), 2);
        assert!(resp.has_more);
        assert_eq!(resp.cursor.last_seq, 2);

        // Continue pulling.
        let resp2 = log.pull(&resp.cursor, 2);
        assert_eq!(resp2.changes.len(), 1);
        assert!(!resp2.has_more);
    }

    #[test]
    fn change_kinds() {
        let log = ChangeLog::new();
        log.append("Todo", "t1", ChangeKind::Insert, Some(serde_json::json!({"title": "Test"})));
        log.append("Todo", "t1", ChangeKind::Update, Some(serde_json::json!({"title": "Updated"})));
        log.append("Todo", "t1", ChangeKind::Delete, None);

        let resp = log.pull(&SyncCursor::beginning(), 100);
        assert_eq!(resp.changes[0].kind, ChangeKind::Insert);
        assert_eq!(resp.changes[1].kind, ChangeKind::Update);
        assert_eq!(resp.changes[2].kind, ChangeKind::Delete);
        assert!(resp.changes[2].data.is_none());
    }

    #[test]
    fn sequence_numbers_are_monotonic() {
        let log = ChangeLog::new();
        let s1 = log.append("A", "1", ChangeKind::Insert, None);
        let s2 = log.append("B", "2", ChangeKind::Insert, None);
        let s3 = log.append("C", "3", ChangeKind::Insert, None);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn serialization_roundtrip() {
        let event = ChangeEvent {
            seq: 1,
            entity: "User".into(),
            row_id: "u1".into(),
            kind: ChangeKind::Insert,
            data: Some(serde_json::json!({"name": "Test"})),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: ChangeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    // -- Edge cases --

    #[test]
    fn pull_from_future_cursor_returns_empty() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        let resp = log.pull(&SyncCursor { last_seq: 999 }, 100);
        assert!(resp.changes.is_empty());
        assert!(!resp.has_more);
    }

    #[test]
    fn pull_limit_zero_returns_empty() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Insert, None);
        let resp = log.pull(&SyncCursor::beginning(), 0);
        assert!(resp.changes.is_empty());
    }

    #[test]
    fn delete_event_has_no_data() {
        let log = ChangeLog::new();
        log.append("User", "u1", ChangeKind::Delete, None);
        let resp = log.pull(&SyncCursor::beginning(), 100);
        assert!(resp.changes[0].data.is_none());
    }

    #[test]
    fn concurrent_appends_get_unique_seqs() {
        let log = ChangeLog::new();
        let s1 = log.append("A", "1", ChangeKind::Insert, None);
        let s2 = log.append("A", "1", ChangeKind::Update, None);
        let s3 = log.append("A", "1", ChangeKind::Delete, None);
        assert!(s1 < s2);
        assert!(s2 < s3);
    }

    #[test]
    fn push_request_serialization() {
        let req = PushRequest {
            changes: vec![
                ClientChange {
                    entity: "User".into(),
                    row_id: "u1".into(),
                    kind: ChangeKind::Insert,
                    data: Some(serde_json::json!({"name": "Alice"})),
                },
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: PushRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].entity, "User");
    }
}
