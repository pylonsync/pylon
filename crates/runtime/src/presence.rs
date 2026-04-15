use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A user's presence state.
#[derive(Debug, Clone)]
pub struct PresenceEntry {
    pub user_id: String,
    pub room: String,
    pub data: serde_json::Value,
    pub last_seen: Instant,
}

/// Server-side presence tracker.
///
/// Tracks which users are currently present in which rooms, with automatic
/// timeout-based expiration. Thread-safe via interior `Mutex`.
pub struct PresenceTracker {
    entries: Mutex<HashMap<String, PresenceEntry>>,
    timeout: Duration,
}

impl PresenceTracker {
    /// Create a new tracker with the given timeout in seconds.
    /// Entries not refreshed within this window are considered stale.
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Upsert a user's presence in a room. Resets the `last_seen` timestamp.
    pub fn set(&self, room: &str, user_id: &str, data: serde_json::Value) {
        let key = format!("{room}:{user_id}");
        let entry = PresenceEntry {
            user_id: user_id.to_string(),
            room: room.to_string(),
            data,
            last_seen: Instant::now(),
        };
        self.entries
            .lock()
            .expect("presence lock poisoned")
            .insert(key, entry);
    }

    /// Explicitly remove a user from a room.
    pub fn remove(&self, room: &str, user_id: &str) {
        let key = format!("{room}:{user_id}");
        self.entries
            .lock()
            .expect("presence lock poisoned")
            .remove(&key);
    }

    /// Return all active (non-timed-out) users in a room.
    pub fn get_room(&self, room: &str) -> Vec<PresenceEntry> {
        let now = Instant::now();
        let entries = self.entries.lock().expect("presence lock poisoned");
        entries
            .values()
            .filter(|e| e.room == room && now.duration_since(e.last_seen) < self.timeout)
            .cloned()
            .collect()
    }

    /// Remove all entries whose `last_seen` exceeds the timeout.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let timeout = self.timeout;
        self.entries
            .lock()
            .expect("presence lock poisoned")
            .retain(|_, e| now.duration_since(e.last_seen) < timeout);
    }

    /// Check whether a specific user is present (and not timed out) in a room.
    pub fn is_present(&self, room: &str, user_id: &str) -> bool {
        let key = format!("{room}:{user_id}");
        let now = Instant::now();
        let entries = self.entries.lock().expect("presence lock poisoned");
        entries
            .get(&key)
            .map(|e| now.duration_since(e.last_seen) < self.timeout)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_room() {
        let tracker = PresenceTracker::new(60);
        tracker.set("lobby", "alice", serde_json::json!({"status": "online"}));
        tracker.set("lobby", "bob", serde_json::json!({"status": "away"}));

        let members = tracker.get_room("lobby");
        assert_eq!(members.len(), 2);

        let user_ids: Vec<&str> = members.iter().map(|e| e.user_id.as_str()).collect();
        assert!(user_ids.contains(&"alice"));
        assert!(user_ids.contains(&"bob"));
    }

    #[test]
    fn get_room_excludes_other_rooms() {
        let tracker = PresenceTracker::new(60);
        tracker.set("lobby", "alice", serde_json::json!({}));
        tracker.set("kitchen", "bob", serde_json::json!({}));

        let lobby = tracker.get_room("lobby");
        assert_eq!(lobby.len(), 1);
        assert_eq!(lobby[0].user_id, "alice");

        let kitchen = tracker.get_room("kitchen");
        assert_eq!(kitchen.len(), 1);
        assert_eq!(kitchen[0].user_id, "bob");
    }

    #[test]
    fn upsert_refreshes_data() {
        let tracker = PresenceTracker::new(60);
        tracker.set("lobby", "alice", serde_json::json!({"status": "online"}));
        tracker.set("lobby", "alice", serde_json::json!({"status": "away"}));

        let members = tracker.get_room("lobby");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].data, serde_json::json!({"status": "away"}));
    }

    #[test]
    fn remove_explicit() {
        let tracker = PresenceTracker::new(60);
        tracker.set("lobby", "alice", serde_json::json!({}));
        assert!(tracker.is_present("lobby", "alice"));

        tracker.remove("lobby", "alice");
        assert!(!tracker.is_present("lobby", "alice"));
        assert!(tracker.get_room("lobby").is_empty());
    }

    #[test]
    fn is_present_returns_false_for_unknown() {
        let tracker = PresenceTracker::new(60);
        assert!(!tracker.is_present("lobby", "nobody"));
    }

    #[test]
    fn timeout_expires_entries() {
        // Use a zero-second timeout so entries expire immediately.
        let tracker = PresenceTracker::new(0);
        tracker.set("lobby", "alice", serde_json::json!({}));

        // Even though we just inserted, a 0s timeout means last_seen is
        // already >= timeout (Duration comparison is strictly less-than).
        assert!(!tracker.is_present("lobby", "alice"));
        assert!(tracker.get_room("lobby").is_empty());
    }

    #[test]
    fn cleanup_removes_stale_entries() {
        let tracker = PresenceTracker::new(0);
        tracker.set("lobby", "alice", serde_json::json!({}));
        tracker.set("lobby", "bob", serde_json::json!({}));

        tracker.cleanup();

        // After cleanup, the internal map should be empty.
        let entries = tracker.entries.lock().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn multiple_rooms_same_user() {
        let tracker = PresenceTracker::new(60);
        tracker.set("lobby", "alice", serde_json::json!({"room": "lobby"}));
        tracker.set("kitchen", "alice", serde_json::json!({"room": "kitchen"}));

        assert!(tracker.is_present("lobby", "alice"));
        assert!(tracker.is_present("kitchen", "alice"));

        tracker.remove("lobby", "alice");
        assert!(!tracker.is_present("lobby", "alice"));
        assert!(tracker.is_present("kitchen", "alice"));
    }
}
