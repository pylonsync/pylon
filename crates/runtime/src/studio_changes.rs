//! The operator change stream: `GET /admin/changes/stream`.
//!
//! Studio's Live sync page tails the change log through this Server-Sent
//! Events stream. It is on the admin surface, so it takes the same credential
//! as the other `/admin/*` routes (the admin token, an admin session, or a
//! Studio operator session). The app's sync WebSocket is not involved and
//! does not accept an operator session.
//!
//! Each connection is one thread that polls the in-memory change log. It
//! holds no database connection and no queue: a slow client blocks only its
//! own thread, and a client that falls behind the retained window gets a
//! `resync` event and continues from the head. [`MAX_STREAMS`] caps the
//! connections.
//!
//! A change carries the row's fields with `serverOnly` and `encrypted` values
//! replaced by [`REDACTED`]. Keys that are not in the manifest are left out.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_sync::{ChangeEvent, ChangeLog, PullError, SyncCursor};

/// The most operator streams the server serves at one time.
pub(crate) const MAX_STREAMS: usize = 8;
/// How often a stream reads the change log.
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// The most changes one read sends.
const BATCH: usize = 200;
/// A comment goes out after this long with no change, so a closed connection
/// is noticed and a proxy does not time the stream out.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// A stream ends after this long. The browser connects again.
const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// What a `serverOnly` or `encrypted` value shows as.
pub(crate) const REDACTED: &str = "[redacted]";

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// One of the [`MAX_STREAMS`] places. Dropping it frees the place.
pub(crate) struct StreamSlot(());

impl StreamSlot {
    /// Take a place, or `None` when every place is taken.
    pub(crate) fn acquire() -> Option<Self> {
        ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_STREAMS).then_some(n + 1)
            })
            .ok()
            .map(|_| StreamSlot(()))
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The fields of a changed row that an operator may see.
///
/// A field the manifest marks `serverOnly` or `encrypted` keeps its key and
/// shows [`REDACTED`]. A key that is not a field of the entity is left out,
/// which covers `passwordHash` and the `_`-prefixed framework columns.
pub(crate) fn redacted_fields(
    manifest: &AppManifest,
    entity: &str,
    data: Option<&serde_json::Value>,
) -> serde_json::Value {
    let (Some(def), Some(serde_json::Value::Object(row))) =
        (manifest.entities.iter().find(|e| e.name == entity), data)
    else {
        return serde_json::Value::Null;
    };
    let mut out = serde_json::Map::new();
    if let Some(id) = row.get("id") {
        out.insert("id".into(), id.clone());
    }
    for field in &def.fields {
        let Some(value) = row.get(&field.name) else {
            continue;
        };
        let shown = if field.server_only || field.encrypted {
            serde_json::Value::String(REDACTED.into())
        } else {
            value.clone()
        };
        out.insert(field.name.clone(), shown);
    }
    serde_json::Value::Object(out)
}

/// One change as a Server-Sent Events frame.
pub(crate) fn change_frame(manifest: &AppManifest, event: &ChangeEvent) -> String {
    let data = serde_json::json!({
        "seq": event.seq,
        "entity": event.entity,
        "row_id": event.row_id,
        "kind": event.kind,
        "timestamp": event.timestamp,
        "fields": redacted_fields(manifest, &event.entity, event.data.as_ref()),
    });
    format!("id: {}\nevent: change\ndata: {data}\n\n", event.seq)
}

fn status_frame(name: &str, log: &ChangeLog) -> String {
    let data = serde_json::json!({
        "head_seq": log.current_seq(),
        "retained": log.len(),
    });
    format!("event: {name}\ndata: {data}\n\n")
}

/// Send changes until the client goes away or the stream reaches
/// [`MAX_LIFETIME`]. `send` writes one chunk to the client.
/// Check current session validity and admin access before each poll.
///
/// A new stream starts at the head of the log: it sends the changes made
/// after the connection opened. A reconnect passes the `Last-Event-ID` the
/// browser sent as `resume_after`, and the stream continues after that
/// sequence number. When the log no longer holds those changes, the first
/// pull sends a `resync` event, so the page knows that it missed changes.
pub(crate) fn run(
    log: &ChangeLog,
    manifest: &AppManifest,
    resume_after: Option<u64>,
    mut authorized: impl FnMut() -> bool,
    mut send: impl FnMut(&[u8]) -> std::io::Result<()>,
) {
    if !authorized() {
        return;
    }
    let started = Instant::now();
    let head = log.current_seq();
    let mut after = match resume_after {
        Some(seq) if seq <= head => seq,
        _ => head,
    };
    let mut last_write = Instant::now();
    if send(format!("retry: 2000\n\n{}", status_frame("hello", log)).as_bytes()).is_err() {
        return;
    }
    while started.elapsed() < MAX_LIFETIME && authorized() {
        let mut out = String::new();
        match log.pull(&SyncCursor { last_seq: after }, BATCH) {
            Ok(page) => {
                for event in &page.changes {
                    out.push_str(&change_frame(manifest, event));
                    after = after.max(event.seq);
                }
            }
            Err(PullError::ResyncRequired { .. }) => {
                // The client is behind the retained window. It continues
                // from the head and knows that it missed changes.
                after = log.current_seq();
                out.push_str(&status_frame("resync", log));
            }
        }
        if out.is_empty() && last_write.elapsed() >= KEEP_ALIVE {
            out.push_str(&status_frame("status", log));
        }
        if !out.is_empty() {
            if send(out.as_bytes()).is_err() {
                return;
            }
            last_write = Instant::now();
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestEntity, ManifestField};
    use pylon_sync::ChangeKind;

    fn manifest() -> AppManifest {
        let field = |name: &str, server_only: bool, encrypted: bool| ManifestField {
            name: name.into(),
            field_type: "string".into(),
            server_only,
            encrypted,
            ..Default::default()
        };
        AppManifest {
            entities: vec![ManifestEntity {
                name: "Org".into(),
                fields: vec![
                    field("name", false, false),
                    field("stripeCustomerId", true, false),
                    field("taxId", false, true),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn revoked_session_stops_before_the_next_change() {
        let log = ChangeLog::new();
        let sessions = pylon_auth::SessionStore::new();
        let session = sessions.create("stream-user".into());
        let mut sent = String::new();
        run(
            &log,
            &manifest(),
            None,
            || sessions.get(&session.token).is_some(),
            |bytes| {
                sent.push_str(&String::from_utf8_lossy(bytes));
                if sent.contains("event: change") {
                    return Err(std::io::Error::other("unexpected disclosure"));
                }
                sessions.revoke(&session.token);
                log.append("Org", "after-revocation", ChangeKind::Insert, None);
                Ok(())
            },
        );
        assert!(sent.contains("event: hello"));
        assert!(!sent.contains("after-revocation"), "{sent}");
    }

    #[test]
    fn server_only_and_encrypted_values_are_redacted() {
        let row = serde_json::json!({
            "id": "o1",
            "name": "Acme",
            "stripeCustomerId": "cus_secret",
            "taxId": "enc:v1:secret",
            "passwordHash": "$argon2id$secret",
            "_internal": "secret",
        });
        let shown = redacted_fields(&manifest(), "Org", Some(&row));
        assert_eq!(shown["id"], "o1");
        assert_eq!(shown["name"], "Acme");
        assert_eq!(shown["stripeCustomerId"], REDACTED);
        assert_eq!(shown["taxId"], REDACTED);
        assert!(!shown.to_string().contains("secret"), "{shown}");
        assert!(shown.get("passwordHash").is_none());
        assert!(shown.get("_internal").is_none());
    }

    #[test]
    fn an_entity_that_is_not_in_the_manifest_shows_no_fields() {
        let row = serde_json::json!({"id": "x", "token": "secret"});
        assert!(redacted_fields(&manifest(), "Ghost", Some(&row)).is_null());
        assert!(redacted_fields(&manifest(), "Org", None).is_null());
    }

    #[test]
    fn a_change_frame_is_one_sse_event_with_redacted_fields() {
        let log = ChangeLog::new();
        log.append(
            "Org",
            "o1",
            ChangeKind::Update,
            Some(serde_json::json!({"id": "o1", "name": "Acme", "stripeCustomerId": "cus_secret"})),
        );
        let page = log.pull(&SyncCursor { last_seq: 0 }, 10).unwrap();
        let frame = change_frame(&manifest(), &page.changes[0]);
        assert!(frame.starts_with("id: "), "{frame}");
        assert!(frame.contains("\nevent: change\ndata: {"), "{frame}");
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"kind\":\"update\""), "{frame}");
        assert!(frame.contains("\"name\":\"Acme\""), "{frame}");
        assert!(!frame.contains("cus_secret"), "{frame}");
    }

    #[test]
    fn the_stream_sends_hello_then_the_changes_made_after_it_opened() {
        let log = ChangeLog::new();
        log.append("Org", "before", ChangeKind::Insert, None);
        let mut sent = String::new();
        let mut writes = 0;
        run(
            &log,
            &manifest(),
            None,
            || true,
            |bytes| {
                sent.push_str(&String::from_utf8_lossy(bytes));
                writes += 1;
                if writes == 1 {
                    // The first write is the hello. A change arrives after it.
                    log.append("Org", "after", ChangeKind::Insert, None);
                    Ok(())
                } else {
                    // The client goes away after the first change.
                    Err(std::io::Error::other("closed"))
                }
            },
        );
        assert!(sent.contains("event: hello"), "{sent}");
        assert!(sent.contains("\"row_id\":\"after\""), "{sent}");
        assert!(!sent.contains("\"row_id\":\"before\""), "{sent}");
    }

    #[test]
    fn a_reconnect_resumes_after_the_last_event_id() {
        let log = ChangeLog::new();
        let seen = log.append("Org", "seen", ChangeKind::Insert, None);
        // These two land while the browser is disconnected.
        log.append("Org", "missed-1", ChangeKind::Insert, None);
        log.append("Org", "missed-2", ChangeKind::Update, None);
        let mut sent = String::new();
        let mut writes = 0;
        run(
            &log,
            &manifest(),
            Some(seen),
            || true,
            |bytes| {
                sent.push_str(&String::from_utf8_lossy(bytes));
                writes += 1;
                if writes < 2 {
                    Ok(())
                } else {
                    Err(std::io::Error::other("closed"))
                }
            },
        );
        assert!(!sent.contains("\"row_id\":\"seen\""), "{sent}");
        assert!(sent.contains("\"row_id\":\"missed-1\""), "{sent}");
        assert!(sent.contains("\"row_id\":\"missed-2\""), "{sent}");
    }

    #[test]
    fn a_last_event_id_past_the_head_starts_at_the_head() {
        let log = ChangeLog::new();
        log.append("Org", "old", ChangeKind::Insert, None);
        let mut sent = String::new();
        let mut writes = 0;
        run(
            &log,
            &manifest(),
            Some(u64::MAX),
            || true,
            |bytes| {
                sent.push_str(&String::from_utf8_lossy(bytes));
                writes += 1;
                if writes == 1 {
                    log.append("Org", "new", ChangeKind::Insert, None);
                    Ok(())
                } else {
                    Err(std::io::Error::other("closed"))
                }
            },
        );
        assert!(!sent.contains("\"row_id\":\"old\""), "{sent}");
        assert!(sent.contains("\"row_id\":\"new\""), "{sent}");
    }

    #[test]
    fn the_streams_are_capped() {
        let held: Vec<StreamSlot> = std::iter::from_fn(StreamSlot::acquire).collect();
        assert_eq!(held.len(), MAX_STREAMS);
        assert!(StreamSlot::acquire().is_none());
        drop(held);
        assert!(StreamSlot::acquire().is_some());
    }
}
