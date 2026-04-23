use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Room events — the messages exchanged in rooms
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomEvent {
    /// Someone joined the room.
    Join {
        room: String,
        user_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    /// Someone left the room.
    Leave { room: String, user_id: String },
    /// Presence update (cursor position, typing indicator, custom data).
    Presence {
        room: String,
        user_id: String,
        data: serde_json::Value,
    },
    /// Arbitrary broadcast to a room.
    Broadcast {
        room: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sender: Option<String>,
        topic: String,
        data: serde_json::Value,
    },
    /// Room state snapshot (sent on join).
    Snapshot { room: String, peers: Vec<PeerInfo> },
}

/// Info about a peer in a room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    pub user_id: String,
    pub data: serde_json::Value,
    pub joined_at: String,
}

// ---------------------------------------------------------------------------
// Room — a single room's state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RoomMember {
    user_id: String,
    data: serde_json::Value,
    joined_at: String,
    last_active: Instant,
}

#[derive(Debug)]
#[allow(dead_code)]
struct Room {
    name: String,
    members: HashMap<String, RoomMember>,
    created_at: String,
}

impl Room {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            members: HashMap::new(),
            created_at: now_iso(),
        }
    }

    fn peer_infos(&self) -> Vec<PeerInfo> {
        self.members
            .values()
            .map(|m| PeerInfo {
                user_id: m.user_id.clone(),
                data: m.data.clone(),
                joined_at: m.joined_at.clone(),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// RoomManager — manages all rooms
// ---------------------------------------------------------------------------

/// Error returned when a room operation fails.
#[derive(Debug, Clone)]
pub struct RoomError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RoomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// Manages named rooms with membership, presence, and broadcasting.
///
/// Rooms are created lazily on first join and removed when the last member leaves.
/// Each room tracks its members, their ephemeral state (typing, cursors, custom data),
/// and supports topic-scoped broadcasting.
pub struct RoomManager {
    rooms: Mutex<HashMap<String, Room>>,
    /// How long before an inactive member is considered gone.
    idle_timeout: Duration,
    /// Maximum number of rooms that can exist simultaneously.
    max_rooms: usize,
}

/// Default maximum number of concurrent rooms.
const DEFAULT_MAX_ROOMS: usize = 10_000;

impl RoomManager {
    pub fn new(idle_timeout_secs: u64) -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            idle_timeout: Duration::from_secs(idle_timeout_secs),
            max_rooms: DEFAULT_MAX_ROOMS,
        }
    }

    /// Create a RoomManager with a custom maximum room limit.
    pub fn with_max_rooms(idle_timeout_secs: u64, max_rooms: usize) -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            idle_timeout: Duration::from_secs(idle_timeout_secs),
            max_rooms,
        }
    }

    /// Join a room. Creates the room if it doesn't exist.
    /// Returns a snapshot of current peers (before this join) and the join event,
    /// or an error if the room limit has been reached and this would create a new room.
    pub fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(RoomEvent, RoomEvent), RoomError> {
        let mut rooms = self.rooms.lock().unwrap();

        // Check if this join would create a new room and if we're at the limit.
        let room_exists = rooms.contains_key(room);
        if !room_exists && rooms.len() >= self.max_rooms {
            return Err(RoomError {
                code: "ROOM_LIMIT_REACHED".to_string(),
                message: format!(
                    "Maximum number of rooms ({}) reached. Cannot create new room.",
                    self.max_rooms
                ),
            });
        }

        let room_state = rooms
            .entry(room.to_string())
            .or_insert_with(|| Room::new(room));

        let member = RoomMember {
            user_id: user_id.to_string(),
            data: data.clone().unwrap_or(serde_json::Value::Null),
            joined_at: now_iso(),
            last_active: Instant::now(),
        };

        // Snapshot of existing peers (before the new member joins).
        let snapshot = RoomEvent::Snapshot {
            room: room.to_string(),
            peers: room_state.peer_infos(),
        };

        room_state.members.insert(user_id.to_string(), member);

        let join_event = RoomEvent::Join {
            room: room.to_string(),
            user_id: user_id.to_string(),
            data,
        };

        Ok((snapshot, join_event))
    }

    /// Leave a room. Removes the room if it becomes empty.
    /// Returns the leave event, or None if the user wasn't in the room.
    pub fn leave(&self, room: &str, user_id: &str) -> Option<RoomEvent> {
        let mut rooms = self.rooms.lock().unwrap();
        let room_state = rooms.get_mut(room)?;
        room_state.members.remove(user_id)?;

        let event = RoomEvent::Leave {
            room: room.to_string(),
            user_id: user_id.to_string(),
        };

        // Clean up empty rooms.
        if room_state.members.is_empty() {
            rooms.remove(room);
        }

        Some(event)
    }

    /// Update a member's ephemeral state (typing indicator, cursor position, etc.).
    /// Returns a presence event, or None if not in the room.
    pub fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<RoomEvent> {
        let mut rooms = self.rooms.lock().unwrap();
        let room_state = rooms.get_mut(room)?;
        let member = room_state.members.get_mut(user_id)?;

        member.data = data.clone();
        member.last_active = Instant::now();

        Some(RoomEvent::Presence {
            room: room.to_string(),
            user_id: user_id.to_string(),
            data,
        })
    }

    /// Get a member's current ephemeral data.
    pub fn get_presence(&self, room: &str, user_id: &str) -> Option<serde_json::Value> {
        let rooms = self.rooms.lock().unwrap();
        rooms
            .get(room)?
            .members
            .get(user_id)
            .map(|m| m.data.clone())
    }

    /// Broadcast an arbitrary event to a room.
    /// Returns the broadcast event, or None if the room doesn't exist.
    pub fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<RoomEvent> {
        let rooms = self.rooms.lock().unwrap();
        if !rooms.contains_key(room) {
            return None;
        }

        Some(RoomEvent::Broadcast {
            room: room.to_string(),
            sender: sender.map(|s| s.to_string()),
            topic: topic.to_string(),
            data,
        })
    }

    /// List all members currently in a room.
    pub fn members(&self, room: &str) -> Vec<PeerInfo> {
        let rooms = self.rooms.lock().unwrap();
        rooms.get(room).map(|r| r.peer_infos()).unwrap_or_default()
    }

    /// List all active room names.
    pub fn list_rooms(&self) -> Vec<String> {
        let rooms = self.rooms.lock().unwrap();
        rooms.keys().cloned().collect()
    }

    /// Check if a user is in a specific room.
    pub fn is_in_room(&self, room: &str, user_id: &str) -> bool {
        let rooms = self.rooms.lock().unwrap();
        rooms
            .get(room)
            .map(|r| r.members.contains_key(user_id))
            .unwrap_or(false)
    }

    /// Get the number of members in a room.
    pub fn room_size(&self, room: &str) -> usize {
        let rooms = self.rooms.lock().unwrap();
        rooms.get(room).map(|r| r.members.len()).unwrap_or(0)
    }

    /// Remove a user from ALL rooms they're in. Returns leave events.
    pub fn disconnect(&self, user_id: &str) -> Vec<RoomEvent> {
        let mut rooms = self.rooms.lock().unwrap();
        let mut events = Vec::new();
        let mut empty_rooms = Vec::new();

        for (room_name, room_state) in rooms.iter_mut() {
            if room_state.members.remove(user_id).is_some() {
                events.push(RoomEvent::Leave {
                    room: room_name.clone(),
                    user_id: user_id.to_string(),
                });
                if room_state.members.is_empty() {
                    empty_rooms.push(room_name.clone());
                }
            }
        }

        for name in empty_rooms {
            rooms.remove(&name);
        }

        events
    }

    /// Remove idle members from all rooms. Returns leave events for each removed member.
    pub fn cleanup_idle(&self) -> Vec<RoomEvent> {
        let now = Instant::now();
        let timeout = self.idle_timeout;
        let mut rooms = self.rooms.lock().unwrap();
        let mut events = Vec::new();
        let mut empty_rooms = Vec::new();

        for (room_name, room_state) in rooms.iter_mut() {
            let idle_users: Vec<String> = room_state
                .members
                .iter()
                .filter(|(_, m)| now.duration_since(m.last_active) >= timeout)
                .map(|(uid, _)| uid.clone())
                .collect();

            for uid in idle_users {
                room_state.members.remove(&uid);
                events.push(RoomEvent::Leave {
                    room: room_name.clone(),
                    user_id: uid,
                });
            }

            if room_state.members.is_empty() {
                empty_rooms.push(room_name.clone());
            }
        }

        for name in empty_rooms {
            rooms.remove(&name);
        }

        events
    }

    /// Get all rooms a user is currently in.
    pub fn user_rooms(&self, user_id: &str) -> Vec<String> {
        let rooms = self.rooms.lock().unwrap();
        rooms
            .iter()
            .filter(|(_, r)| r.members.contains_key(user_id))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

fn now_iso() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{ts}Z")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn join_creates_room() {
        let mgr = RoomManager::new(60);
        assert!(mgr.list_rooms().is_empty());

        let (snapshot, join) = mgr.join("lobby", "alice", None).unwrap();

        // Snapshot should be empty (alice is the first member).
        assert!(matches!(snapshot, RoomEvent::Snapshot { ref peers, .. } if peers.is_empty()));
        assert!(matches!(join, RoomEvent::Join { ref user_id, .. } if user_id == "alice"));
        assert_eq!(mgr.list_rooms(), vec!["lobby"]);
        assert_eq!(mgr.room_size("lobby"), 1);
    }

    #[test]
    fn join_returns_existing_peers_in_snapshot() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", Some(serde_json::json!({"color": "red"})))
            .unwrap();

        let (snapshot, _) = mgr.join("lobby", "bob", None).unwrap();
        if let RoomEvent::Snapshot { peers, .. } = snapshot {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].user_id, "alice");
            assert_eq!(peers[0].data, serde_json::json!({"color": "red"}));
        } else {
            panic!("expected Snapshot");
        }
    }

    #[test]
    fn leave_removes_member() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", None).unwrap();
        mgr.join("lobby", "bob", None).unwrap();

        let event = mgr.leave("lobby", "alice");
        assert!(event.is_some());
        assert!(!mgr.is_in_room("lobby", "alice"));
        assert!(mgr.is_in_room("lobby", "bob"));
        assert_eq!(mgr.room_size("lobby"), 1);
    }

    #[test]
    fn leave_last_member_removes_room() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", None).unwrap();

        mgr.leave("lobby", "alice");
        assert!(mgr.list_rooms().is_empty());
        assert_eq!(mgr.room_size("lobby"), 0);
    }

    #[test]
    fn leave_nonexistent_returns_none() {
        let mgr = RoomManager::new(60);
        assert!(mgr.leave("lobby", "alice").is_none());
    }

    #[test]
    fn set_and_get_presence() {
        let mgr = RoomManager::new(60);
        mgr.join("doc:123", "alice", None).unwrap();

        let event = mgr.set_presence(
            "doc:123",
            "alice",
            serde_json::json!({"cursor": {"x": 10, "y": 20}}),
        );
        assert!(event.is_some());

        let data = mgr.get_presence("doc:123", "alice").unwrap();
        assert_eq!(data, serde_json::json!({"cursor": {"x": 10, "y": 20}}));
    }

    #[test]
    fn presence_not_in_room_returns_none() {
        let mgr = RoomManager::new(60);
        assert!(mgr
            .set_presence("lobby", "alice", serde_json::json!({}))
            .is_none());
        assert!(mgr.get_presence("lobby", "alice").is_none());
    }

    #[test]
    fn broadcast_to_room() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", None).unwrap();

        let event = mgr.broadcast(
            "lobby",
            Some("alice"),
            "typing",
            serde_json::json!({"active": true}),
        );
        assert!(event.is_some());

        if let Some(RoomEvent::Broadcast {
            topic,
            sender,
            data,
            ..
        }) = event
        {
            assert_eq!(topic, "typing");
            assert_eq!(sender, Some("alice".to_string()));
            assert_eq!(data, serde_json::json!({"active": true}));
        } else {
            panic!("expected Broadcast");
        }
    }

    #[test]
    fn broadcast_to_nonexistent_room() {
        let mgr = RoomManager::new(60);
        assert!(mgr
            .broadcast("ghost", None, "ping", serde_json::json!({}))
            .is_none());
    }

    #[test]
    fn members_list() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", Some(serde_json::json!({"role": "admin"})))
            .unwrap();
        mgr.join("lobby", "bob", None).unwrap();

        let members = mgr.members("lobby");
        assert_eq!(members.len(), 2);

        let ids: HashSet<String> = members.iter().map(|m| m.user_id.clone()).collect();
        assert!(ids.contains("alice"));
        assert!(ids.contains("bob"));
    }

    #[test]
    fn disconnect_removes_from_all_rooms() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", None).unwrap();
        mgr.join("kitchen", "alice", None).unwrap();
        mgr.join("lobby", "bob", None).unwrap();

        let events = mgr.disconnect("alice");
        assert_eq!(events.len(), 2);
        assert!(!mgr.is_in_room("lobby", "alice"));
        assert!(!mgr.is_in_room("kitchen", "alice"));
        assert!(mgr.is_in_room("lobby", "bob"));
        // Kitchen should be removed (was only alice).
        assert!(!mgr.list_rooms().contains(&"kitchen".to_string()));
    }

    #[test]
    fn cleanup_idle_members() {
        let mgr = RoomManager::new(0); // 0 timeout = immediate expiry
        mgr.join("lobby", "alice", None).unwrap();
        mgr.join("lobby", "bob", None).unwrap();

        let events = mgr.cleanup_idle();
        assert_eq!(events.len(), 2);
        assert!(mgr.list_rooms().is_empty());
    }

    #[test]
    fn user_rooms() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", None).unwrap();
        mgr.join("kitchen", "alice", None).unwrap();
        mgr.join("lobby", "bob", None).unwrap();

        let mut rooms = mgr.user_rooms("alice");
        rooms.sort();
        assert_eq!(rooms, vec!["kitchen", "lobby"]);
        assert_eq!(mgr.user_rooms("bob"), vec!["lobby"]);
        assert!(mgr.user_rooms("nobody").is_empty());
    }

    #[test]
    fn entity_scoped_room() {
        let mgr = RoomManager::new(60);
        mgr.join(
            "Todo:t1",
            "alice",
            Some(serde_json::json!({"editing": true})),
        )
        .unwrap();
        mgr.join("Todo:t1", "bob", Some(serde_json::json!({"viewing": true})))
            .unwrap();

        assert_eq!(mgr.room_size("Todo:t1"), 2);

        mgr.set_presence("Todo:t1", "alice", serde_json::json!({"cursor": 42}));
        let data = mgr.get_presence("Todo:t1", "alice").unwrap();
        assert_eq!(data, serde_json::json!({"cursor": 42}));
    }

    #[test]
    fn rejoin_updates_data() {
        let mgr = RoomManager::new(60);
        mgr.join("lobby", "alice", Some(serde_json::json!({"v": 1})))
            .unwrap();
        mgr.join("lobby", "alice", Some(serde_json::json!({"v": 2})))
            .unwrap();

        // Should still be 1 member, not 2.
        assert_eq!(mgr.room_size("lobby"), 1);
        let members = mgr.members("lobby");
        assert_eq!(members[0].data, serde_json::json!({"v": 2}));
    }

    #[test]
    fn room_event_serialization() {
        let event = RoomEvent::Join {
            room: "lobby".into(),
            user_id: "alice".into(),
            data: Some(serde_json::json!({"color": "red"})),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"join\""));
        assert!(json.contains("\"room\":\"lobby\""));

        let parsed: RoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn broadcast_event_serialization() {
        let event = RoomEvent::Broadcast {
            room: "lobby".into(),
            sender: None,
            topic: "system".into(),
            data: serde_json::json!({"msg": "hello"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"broadcast\""));
        assert!(!json.contains("\"sender\"")); // skip_serializing_if None

        let parsed: RoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    // --- Room limit tests ---

    #[test]
    fn max_rooms_enforced() {
        let mgr = RoomManager::with_max_rooms(60, 2);
        mgr.join("room1", "alice", None).unwrap();
        mgr.join("room2", "bob", None).unwrap();

        // Third distinct room should fail.
        let result = mgr.join("room3", "charlie", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "ROOM_LIMIT_REACHED");
        assert!(err.message.contains("2"));
    }

    #[test]
    fn joining_existing_room_at_limit_succeeds() {
        let mgr = RoomManager::with_max_rooms(60, 2);
        mgr.join("room1", "alice", None).unwrap();
        mgr.join("room2", "bob", None).unwrap();

        // Joining an existing room should still work even at the limit.
        let result = mgr.join("room1", "charlie", None);
        assert!(result.is_ok());
        assert_eq!(mgr.room_size("room1"), 2);
    }

    #[test]
    fn room_limit_freed_after_leave() {
        let mgr = RoomManager::with_max_rooms(60, 2);
        mgr.join("room1", "alice", None).unwrap();
        mgr.join("room2", "bob", None).unwrap();

        // At limit.
        assert!(mgr.join("room3", "charlie", None).is_err());

        // Free up a slot by having last member leave.
        mgr.leave("room2", "bob");

        // Now we can create a new room.
        assert!(mgr.join("room3", "charlie", None).is_ok());
    }

    #[test]
    fn default_max_rooms_is_10000() {
        let mgr = RoomManager::new(60);
        assert_eq!(mgr.max_rooms, DEFAULT_MAX_ROOMS);
    }
}
