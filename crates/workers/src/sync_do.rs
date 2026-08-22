//! `PylonSync` — the Durable Object sync relay
//! (docs/SYNC_DURABLE_OBJECTS_DESIGN.md, Option C).
//!
//! The machine owns the change log and `seq`; this DO owns DELIVERY:
//! hibernating client WebSockets, a bounded recent-event ring in DO
//! storage, and per-subscriber policy filtering via
//! [`crate::relay_core::RelayFilter`]. It holds no truth — a lost
//! event degrades to a catch-up read against the machine.
//!
//! One DO instance per app (`id_from_name(app)`), routed by the worker
//! fetch handler under `/sync/*?app=...`:
//!
//! - `POST /sync/manifest` (machine, HMAC) — `{"manifest":…,
//!   "auth_user":…}`. Stored; the `PolicyEngine` compiles from it.
//!   Until it arrives the DO fails CLOSED: sockets are accepted but no
//!   frame is delivered.
//! - `POST /sync/push` (machine, HMAC) — `{"events":[ChangeEvent…]}`
//!   with RAW `data`/`prev_data`. Ring-buffered + fanned out filtered.
//! - `GET /sync/ws?app=…[&since=<seq>]` (client) — WebSocket upgrade.
//!   The signed auth blob (`pylon_auth::relay_blob`) rides the
//!   `bearer.<blob>` subprotocol, NOT the URL (keeps the credential out
//!   of proxy logs). The blob is app-bound: the DO rejects one minted
//!   for a different app. `since` replays the ring tail after that seq
//!   before live frames; a cursor older than the ring closes with 4410
//!   so the client falls back to `/api/sync/pull`.
//! - `POST /sync/cluster/push` + `GET /sync/cluster/ws` (machines,
//!   HMAC) — raw origin-to-origin cluster envelopes. This is the managed
//!   Pylon Cloud transport for changes, presence, sessions, and CRDT frames.
//!
//! Auth blobs expire (default 15 min): the fan-out path closes expired
//! sockets with 4401 so clients re-handshake against the machine —
//! that's the staleness bound on revoked roles.
//!
//! Everything here is async with ZERO `block_on` — `block_on` on
//! wasm32 can't yield to the JS event loop and hangs at runtime even
//! though `cargo check` passes (see crates/workers/README.md).

#![cfg(feature = "workers")]

use std::cell::RefCell;
use std::collections::HashMap;

use worker::{
    durable_object, Date, DurableObject, Env, Method, Request, Response, Result, State,
    WebSocketPair,
};

use crate::relay_core::{
    cluster_event_key, event_key, parse_cluster_event_key, parse_event_key, ClusterRing, EventRing,
    RelayFilter, CLUSTER_RING_CAPACITY, RING_CAPACITY,
};
use pylon_auth::relay_blob::RelayAuthClaims;
use pylon_cluster::{Envelope, RelayFrame};
use pylon_sync::ChangeEvent;

/// Socket close codes (application range 4000-4999).
const CLOSE_TOKEN_EXPIRED: u16 = 4401;
const CLOSE_RESYNC_REQUIRED: u16 = 4410;
const CLOSE_CLUSTER_RESYNC_REQUIRED: u16 = 4411;

const MANIFEST_KEY: &str = "mf";

#[durable_object]
pub struct PylonSync {
    state: State,
    env: Env,
    /// Compiled policy filter. `None` until the machine's manifest
    /// push arrives (fail closed) or after a hibernation wake before
    /// [`Self::hydrate`] restores it from storage.
    filter: RefCell<Option<RelayFilter>>,
    ring: RefCell<EventRing>,
    cluster_ring: RefCell<ClusterRing>,
    /// Per-connection verified claims, keyed by the connection id
    /// carried in the socket's hibernation tag (`c:<id>`). Mirrored in
    /// storage under `conn:<id>` so a wake can re-resolve a socket.
    conns: RefCell<HashMap<String, RelayAuthClaims>>,
    conn_counter: RefCell<u32>,
    hydrated: RefCell<bool>,
}

#[durable_object]
impl DurableObject for PylonSync {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            filter: RefCell::new(None),
            ring: RefCell::new(EventRing::default()),
            cluster_ring: RefCell::new(ClusterRing::default()),
            conns: RefCell::new(HashMap::new()),
            conn_counter: RefCell::new(0),
            hydrated: RefCell::new(false),
        }
    }

    async fn fetch(&mut self, req: Request) -> Result<Response> {
        let mut req = req;
        self.hydrate().await;
        let path = req.path();
        let method = req.method();
        match (method, path.as_str()) {
            (Method::Post, "/sync/manifest") => self.handle_manifest(&mut req).await,
            (Method::Post, "/sync/push") => self.handle_push(&mut req).await,
            (Method::Post, "/sync/cluster/push") => self.handle_cluster_push(&mut req).await,
            (Method::Get, "/sync/cluster/ws") => self.handle_cluster_ws(&req).await,
            (Method::Get, "/sync/ws") => self.handle_ws(&req).await,
            (Method::Get, "/sync/status") => self.handle_status(&mut req).await,
            _ => Response::error("not found", 404),
        }
    }

    async fn websocket_message(
        &mut self,
        ws: worker::WebSocket,
        msg: worker::WebSocketIncomingMessage,
    ) -> Result<()> {
        self.hydrate().await;
        let text = match msg {
            worker::WebSocketIncomingMessage::String(s) => s,
            worker::WebSocketIncomingMessage::Binary(_) => return Ok(()),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        match parsed.get("type").and_then(|t| t.as_str()) {
            Some("ping") => {
                let _ = ws.send_with_str(r#"{"type":"pong"}"#);
            }
            // In-band catch-up without reconnecting: replay the ring
            // tail after `since`, filtered for this subscriber.
            Some("pull") => {
                let since = parsed.get("since").and_then(|s| s.as_u64()).unwrap_or(0);
                if let Some(claims) = self.claims_for(&ws).await {
                    if claims.exp <= now_secs() {
                        let _ = ws.close(Some(CLOSE_TOKEN_EXPIRED), Some("relay token expired"));
                        return Ok(());
                    }
                    self.replay_since(&ws, &claims, since);
                }
            }
            // Everything else (room/crdt/reactive subscriptions) is a
            // machine-WS feature the relay doesn't carry. Ignored, not
            // an error — the SDK replays them blindly on connect.
            _ => {}
        }
        Ok(())
    }

    async fn websocket_close(
        &mut self,
        ws: worker::WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        if let Some(id) = conn_id_of(&self.state, &ws) {
            self.conns.borrow_mut().remove(&id);
            let _ = self.state.storage().delete(&format!("conn:{id}")).await;
        }
        Ok(())
    }
}

fn now_secs() -> u64 {
    Date::now().as_millis() / 1000
}

fn conn_id_of(state: &State, ws: &worker::WebSocket) -> Option<String> {
    state
        .get_tags(ws)
        .into_iter()
        .find_map(|t| t.strip_prefix("c:").map(str::to_string))
}

fn origin_id_of(state: &State, ws: &worker::WebSocket) -> Option<String> {
    state
        .get_tags(ws)
        .into_iter()
        .find_map(|t| t.strip_prefix("o:").map(str::to_string))
}

/// The app this request was routed to (the worker's `?app=` param).
/// Every signed payload and every auth blob is bound to an app; the DO
/// rejects a mismatch, which is what stops a cross-app replay on a
/// shared-secret worker.
fn routed_app(req: &Request) -> String {
    req.url()
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "app")
                .map(|(_, v)| v.to_string())
        })
        .unwrap_or_default()
}

impl PylonSync {
    /// Restore filter, rings, and connection claims after a hibernation
    /// wake. Storage layout: `mf` (manifest payload string),
    /// `e:{seq:020}` (event JSON), `conn:{id}` (claims JSON).
    async fn hydrate(&self) {
        if *self.hydrated.borrow() {
            return;
        }
        // Flag flips only AFTER the awaited loads below complete — set
        // early and a concurrent handler could observe an empty ring /
        // conn map (skipping a live socket, or clobbering a just-pushed
        // event). The loads are idempotent, so a double-hydrate before
        // the flag flips is harmless; a premature "done" is not.
        let storage = self.state.storage();
        if let Ok(payload) = storage.get::<String>(MANIFEST_KEY).await {
            match RelayFilter::from_manifest_payload(&payload) {
                Ok(f) => *self.filter.borrow_mut() = Some(f),
                Err(e) => worker::console_log!("[pylon-sync] stored manifest unusable: {e}"),
            }
        }
        let Ok(map) = storage.list().await else {
            return;
        };
        let events: RefCell<Vec<ChangeEvent>> = RefCell::new(Vec::new());
        let cluster_frames: RefCell<Vec<RelayFrame>> = RefCell::new(Vec::new());
        let conns: RefCell<Vec<(String, RelayAuthClaims)>> = RefCell::new(Vec::new());
        map.for_each(&mut |value, key| {
            let Some(key_str) = key.as_string() else {
                return;
            };
            if parse_event_key(&key_str).is_some() {
                if let Ok(s) = serde_wasm_bindgen::from_value::<String>(value) {
                    if let Ok(ev) = serde_json::from_str::<ChangeEvent>(&s) {
                        events.borrow_mut().push(ev);
                    }
                }
            } else if parse_cluster_event_key(&key_str).is_some() {
                if let Ok(s) = serde_wasm_bindgen::from_value::<String>(value) {
                    if let Ok(frame) = serde_json::from_str::<RelayFrame>(&s) {
                        cluster_frames.borrow_mut().push(frame);
                    }
                }
            } else if let Some(id) = key_str.strip_prefix("conn:") {
                if let Ok(s) = serde_wasm_bindgen::from_value::<String>(value) {
                    if let Ok(claims) = serde_json::from_str::<RelayAuthClaims>(&s) {
                        conns.borrow_mut().push((id.to_string(), claims));
                    }
                }
            }
        });
        *self.ring.borrow_mut() = EventRing::hydrate(events.into_inner());
        *self.cluster_ring.borrow_mut() = ClusterRing::hydrate(cluster_frames.into_inner());
        {
            let mut conn_map = self.conns.borrow_mut();
            for (id, claims) in conns.into_inner() {
                conn_map.insert(id, claims);
            }
        }
        *self.hydrated.borrow_mut() = true;
    }

    fn root_secret(&self) -> Option<String> {
        self.env
            .secret("PYLON_RELAY_SECRET")
            .map(|s| s.to_string())
            .ok()
            .or_else(|| {
                self.env
                    .var("PYLON_RELAY_SECRET")
                    .map(|v| v.to_string())
                    .ok()
            })
            .filter(|s| !s.is_empty())
    }

    fn app_secrets(&self, app: &str) -> Vec<String> {
        let Some(root) = self.root_secret() else {
            return Vec::new();
        };
        let derive = self
            .env
            .var("PYLON_RELAY_DERIVE_APP_SECRETS")
            .map(|v| matches!(v.to_string().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        if derive {
            // Keep the root key valid for existing self-hosted relay users.
            // Pylon Cloud customer machines receive only the derived key.
            vec![pylon_auth::relay_blob::derive_app_secret(&root, app), root]
        } else {
            vec![root]
        }
    }

    /// Verify the machine's HMAC on a signed request and return the
    /// body. 401 on any failure — same verification primitive as the
    /// machine (`pylon_auth::trusted_mint`).
    async fn verified_body(
        &self,
        req: &mut Request,
        app: &str,
    ) -> std::result::Result<String, Response> {
        let secrets = self.app_secrets(app);
        if secrets.is_empty() {
            return Err(Response::error("PYLON_RELAY_SECRET not configured", 503)
                .unwrap_or_else(|_| Response::empty().unwrap()));
        }
        let ts: u64 = req
            .headers()
            .get("X-Pylon-Relay-Timestamp")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let sig = req
            .headers()
            .get("X-Pylon-Relay-Signature")
            .ok()
            .flatten()
            .unwrap_or_default();
        let body = req.text().await.unwrap_or_default();
        let now = now_secs();
        if secrets.iter().any(|secret| {
            pylon_auth::trusted_mint::verify_signature(secret, ts, body.as_bytes(), &sig, now)
                .is_ok()
        }) {
            Ok(body)
        } else {
            Err(Response::error("relay auth failed", 401)
                .unwrap_or_else(|_| Response::empty().unwrap()))
        }
    }

    fn verify_signed_payload(
        &self,
        req: &Request,
        app: &str,
        payload: &[u8],
    ) -> std::result::Result<(), Response> {
        let secrets = self.app_secrets(app);
        if secrets.is_empty() {
            return Err(Response::error("PYLON_RELAY_SECRET not configured", 503)
                .unwrap_or_else(|_| Response::empty().unwrap()));
        }
        let ts: u64 = req
            .headers()
            .get("X-Pylon-Relay-Timestamp")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let sig = req
            .headers()
            .get("X-Pylon-Relay-Signature")
            .ok()
            .flatten()
            .unwrap_or_default();
        let now = now_secs();
        if secrets.iter().any(|secret| {
            pylon_auth::trusted_mint::verify_signature(secret, ts, payload, &sig, now).is_ok()
        }) {
            Ok(())
        } else {
            Err(Response::error("relay auth failed", 401)
                .unwrap_or_else(|_| Response::empty().unwrap()))
        }
    }

    async fn handle_manifest(&self, req: &mut Request) -> Result<Response> {
        let app = routed_app(req);
        let body = match self.verified_body(req, &app).await {
            Ok(b) => b,
            Err(resp) => return Ok(resp),
        };
        // The signed body carries the app it was minted for. Reject a
        // manifest whose app doesn't match this DO's — otherwise a
        // captured/self-minted manifest could be replayed to another
        // app's DO to swap its policies (cross-app leak).
        #[derive(serde::Deserialize)]
        struct AppEnvelope {
            #[serde(default)]
            app: String,
        }
        let env: AppEnvelope =
            serde_json::from_str(&body).unwrap_or(AppEnvelope { app: String::new() });
        if env.app != app {
            return Response::error("manifest app mismatch", 403);
        }
        match RelayFilter::from_manifest_payload(&body) {
            Ok(filter) => {
                let _ = self.state.storage().put(MANIFEST_KEY, body).await;
                *self.filter.borrow_mut() = Some(filter);
                Response::from_json(&serde_json::json!({ "ok": true }))
            }
            Err(e) => Response::error(format!("bad manifest payload: {e}"), 400),
        }
    }

    async fn handle_push(&self, req: &mut Request) -> Result<Response> {
        let app = routed_app(req);
        let body = match self.verified_body(req, &app).await {
            Ok(b) => b,
            Err(resp) => return Ok(resp),
        };
        #[derive(serde::Deserialize)]
        struct Push {
            #[serde(default)]
            app: String,
            events: Vec<ChangeEvent>,
        }
        let push: Push = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return Response::error(format!("bad push payload: {e}"), 400),
        };
        if push.app != app {
            return Response::error("push app mismatch", 403);
        }

        let mut accepted: Vec<ChangeEvent> = Vec::with_capacity(push.events.len());
        let mut evicted_seqs: Vec<u64> = Vec::new();
        {
            let mut ring = self.ring.borrow_mut();
            for ev in push.events {
                if let Some(evicted) = ring.push(ev.clone()) {
                    accepted.push(ev);
                    evicted_seqs.extend(evicted);
                }
            }
        }
        for ev in &accepted {
            if let Ok(json) = serde_json::to_string(ev) {
                let _ = self.state.storage().put(&event_key(ev.seq), json).await;
            }
        }
        for seq in evicted_seqs {
            let _ = self.state.storage().delete(&event_key(seq)).await;
        }

        // Fan out — per-subscriber filtered, expired sockets closed.
        let filter = self.filter.borrow();
        let Some(filter) = filter.as_ref() else {
            // No manifest yet: events are ringed for later catch-up,
            // nothing is delivered (fail closed).
            return Response::from_json(&serde_json::json!({
                "accepted": accepted.len(), "delivered": 0, "filtered": "no-manifest"
            }));
        };
        let now = now_secs();
        let mut delivered = 0usize;
        for ws in self.state.get_websockets() {
            let Some(id) = conn_id_of(&self.state, &ws) else {
                continue;
            };
            let claims = self.conns.borrow().get(&id).cloned();
            let Some(claims) = claims else { continue };
            if claims.exp <= now {
                let _ = ws.close(Some(CLOSE_TOKEN_EXPIRED), Some("relay token expired"));
                continue;
            }
            let auth = claims.into_auth_context();
            for ev in &accepted {
                if let Some(frame) = filter.wire_json_for(&auth, ev) {
                    if ws.send_with_str(&frame).is_ok() {
                        delivered += 1;
                    }
                }
            }
        }
        Response::from_json(&serde_json::json!({
            "accepted": accepted.len(), "delivered": delivered
        }))
    }

    async fn handle_cluster_push(&self, req: &mut Request) -> Result<Response> {
        let app = routed_app(req);
        let body = match self.verified_body(req, &app).await {
            Ok(body) => body,
            Err(resp) => return Ok(resp),
        };
        #[derive(serde::Deserialize)]
        struct Push {
            #[serde(default)]
            app: String,
            #[serde(default)]
            message_id: String,
            envelope: Envelope,
        }
        let push: Push = match serde_json::from_str(&body) {
            Ok(push) => push,
            Err(e) => return Response::error(format!("bad cluster push payload: {e}"), 400),
        };
        if push.app != app {
            return Response::error("cluster push app mismatch", 403);
        }

        if push.message_id.is_empty() || push.message_id.len() > 128 {
            return Response::error("missing or invalid cluster message_id", 400);
        }
        let (frame, inserted, evicted) = self
            .cluster_ring
            .borrow_mut()
            .push(push.envelope, push.message_id);
        if !inserted {
            return Response::from_json(&serde_json::json!({
                "accepted": 0,
                "duplicate": true,
                "relay_seq": frame.relay_seq,
            }));
        }
        if let Ok(json) = serde_json::to_string(&frame) {
            let _ = self
                .state
                .storage()
                .put(&cluster_event_key(frame.relay_seq), json.clone())
                .await;
            let mut delivered = 0usize;
            for ws in self.state.get_websockets() {
                let Some(origin_id) = origin_id_of(&self.state, &ws) else {
                    continue;
                };
                if origin_id == frame.envelope.instance_id {
                    continue;
                }
                if ws.send_with_str(&json).is_ok() {
                    delivered += 1;
                }
            }
            for seq in evicted {
                let _ = self.state.storage().delete(&cluster_event_key(seq)).await;
            }
            return Response::from_json(&serde_json::json!({
                "accepted": 1,
                "relay_seq": frame.relay_seq,
                "delivered": delivered,
            }));
        }
        Response::error("cluster frame serialization failed", 500)
    }

    async fn handle_cluster_ws(&self, req: &Request) -> Result<Response> {
        let app = routed_app(req);
        let url = req.url()?;
        let instance_id = url
            .query_pairs()
            .find(|(key, _)| key == "instance")
            .map(|(_, value)| value.to_string())
            .unwrap_or_default();
        if instance_id.is_empty() || instance_id.len() > 128 {
            return Response::error("missing or invalid instance", 400);
        }
        let signed_payload = format!("{app}.{instance_id}");
        if let Err(resp) = self.verify_signed_payload(req, &app, signed_payload.as_bytes()) {
            return Ok(resp);
        }
        let since = url
            .query_pairs()
            .find(|(key, _)| key == "since")
            .and_then(|(_, value)| value.parse::<u64>().ok());

        let pair = WebSocketPair::new()?;
        self.state
            .accept_websocket_with_tags(&pair.server, &[&format!("o:{instance_id}")]);
        if let Some(since) = since {
            self.replay_cluster_since(&pair.server, since);
        }
        Response::from_websocket(pair.client)
    }

    fn replay_cluster_since(&self, ws: &worker::WebSocket, since: u64) {
        let ring = self.cluster_ring.borrow();
        match ring.since(since) {
            Some(frames) => {
                for frame in frames {
                    if let Ok(json) = serde_json::to_string(frame) {
                        let _ = ws.send_with_str(&json);
                    }
                }
            }
            None => {
                let _ = ws.close(
                    Some(CLOSE_CLUSTER_RESYNC_REQUIRED),
                    Some("cluster cursor older than relay ring"),
                );
            }
        }
    }

    async fn handle_ws(&self, req: &Request) -> Result<Response> {
        let app = routed_app(req);
        let secrets = self.app_secrets(&app);
        if secrets.is_empty() {
            return Response::error("PYLON_RELAY_SECRET not configured", 503);
        }
        let url = req.url()?;
        let since: Option<u64> = url
            .query_pairs()
            .find(|(k, _)| k == "since")
            .and_then(|(_, v)| v.parse().ok());
        // The blob rides in the `bearer.<blob>` WebSocket subprotocol,
        // NOT the URL query: a URL would land in CDN/proxy access logs,
        // and the blob is a live credential. Same envelope as the
        // machine WS's `bearer.<token>`, so TS and Swift reuse their
        // existing bearer path unchanged. Our blob charset (base64url +
        // '.' + hex) needs no percent decoding, but the clients still
        // encodeURIComponent it — a no-op here.
        //
        // Keep the FULL offered protocol string: a browser that offered
        // a subprotocol fails the handshake unless the 101 response
        // selects one, so we must echo it back below. (The machine WS
        // echoes it for the same reason.)
        let offered = req
            .headers()
            .get("Sec-WebSocket-Protocol")
            .ok()
            .flatten()
            .and_then(|hdr| {
                hdr.split(',')
                    .map(|s| s.trim())
                    .find(|p| p.starts_with("bearer."))
                    .map(str::to_string)
            });
        let Some(offered) = offered else {
            return Response::error("missing relay subprotocol", 401);
        };
        let token = offered["bearer.".len()..].to_string();
        let now = now_secs();
        let claims = secrets
            .iter()
            .find_map(|secret| pylon_auth::relay_blob::verify(secret, &app, &token, now).ok());
        let Some(claims) = claims else {
            return Response::error("relay token rejected", 401);
        };

        // Connection id: wall-millis + per-instance counter. DOs are
        // single-threaded, so two upgrades can't race the counter; a
        // hibernation reset can't happen mid-burst, so ms+counter
        // never collides in practice.
        let id = {
            let mut n = self.conn_counter.borrow_mut();
            *n = n.wrapping_add(1);
            format!("{:x}-{:x}", Date::now().as_millis(), *n)
        };

        let pair = WebSocketPair::new()?;
        // Tagged + hibernation-compatible: the tag survives sleep, the
        // claims re-resolve from `conn:<id>` storage on wake.
        self.state
            .accept_websocket_with_tags(&pair.server, &[&format!("c:{id}")]);
        let claims_json = serde_json::to_string(&claims).unwrap_or_default();
        let _ = self
            .state
            .storage()
            .put(&format!("conn:{id}"), claims_json)
            .await;
        self.conns.borrow_mut().insert(id, claims.clone());

        // Cursor catch-up before live frames.
        if let Some(since) = since {
            self.replay_since(&pair.server, &claims, since);
        }
        // Echo the selected subprotocol on the 101 — browsers abort the
        // handshake otherwise. Non-fatal if the header can't be set
        // (native clients tolerate a missing echo).
        let mut resp = Response::from_websocket(pair.client)?;
        let _ = resp.headers_mut().set("Sec-WebSocket-Protocol", &offered);
        Ok(resp)
    }

    /// Claims for a live socket, from the in-memory map or (after a
    /// hibernation wake that raced `hydrate`) from `conn:<id>` storage.
    async fn claims_for(&self, ws: &worker::WebSocket) -> Option<RelayAuthClaims> {
        let id = conn_id_of(&self.state, ws)?;
        if let Some(c) = self.conns.borrow().get(&id) {
            return Some(c.clone());
        }
        let stored: String = self.state.storage().get(&format!("conn:{id}")).await.ok()?;
        let claims: RelayAuthClaims = serde_json::from_str(&stored).ok()?;
        self.conns.borrow_mut().insert(id, claims.clone());
        Some(claims)
    }

    /// Replay ring events after `since` to one socket, filtered. A
    /// cursor below the ring closes 4410 — the client must pull from
    /// the machine (deep history stays on the origin).
    fn replay_since(&self, ws: &worker::WebSocket, claims: &RelayAuthClaims, since: u64) {
        let filter = self.filter.borrow();
        let Some(filter) = filter.as_ref() else {
            return;
        };
        let auth = claims.clone().into_auth_context();
        let ring = self.ring.borrow();
        match ring.since(since) {
            Some(events) => {
                for ev in events {
                    if let Some(frame) = filter.wire_json_for(&auth, ev) {
                        let _ = ws.send_with_str(&frame);
                    }
                }
            }
            None => {
                let _ = ws.close(
                    Some(CLOSE_RESYNC_REQUIRED),
                    Some("cursor older than relay ring; pull from origin"),
                );
            }
        }
    }

    /// Operational metadata (ring watermarks, socket count). HMAC-gated
    /// like push/manifest — an open endpoint would leak per-app seq
    /// watermarks and connection counts to any caller on a shared
    /// worker.
    async fn handle_status(&self, req: &mut Request) -> Result<Response> {
        let app = routed_app(req);
        if let Err(resp) = self.verified_body(req, &app).await {
            return Ok(resp);
        }
        let ring = self.ring.borrow();
        let cluster_ring = self.cluster_ring.borrow();
        let sockets = self.state.get_websockets();
        let origin_sockets = sockets
            .iter()
            .filter(|ws| origin_id_of(&self.state, ws).is_some())
            .count();
        Response::from_json(&serde_json::json!({
            "has_manifest": self.filter.borrow().is_some(),
            "ring_len": ring.len(),
            "oldest_seq": ring.oldest_seq(),
            "latest_seq": ring.latest_seq(),
            "ring_capacity": RING_CAPACITY,
            "sockets": sockets.len(),
            "cluster": {
                "ring_len": cluster_ring.len(),
                "oldest_seq": cluster_ring.oldest_seq(),
                "latest_seq": cluster_ring.latest_seq(),
                "ring_capacity": CLUSTER_RING_CAPACITY,
                "origin_sockets": origin_sockets,
            },
        }))
    }
}
