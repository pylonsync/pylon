//! `PylonRoom` Durable Object class + WorkersRooms `RoomOps`
//! adapter.
//!
//! Rooms map one-to-one onto Durable Objects. Each room name
//! hashes deterministically to one DO instance via
//! `env.durable_object("PYLON_ROOMS").id_from_name(room_name)` —
//! Cloudflare routes every request for that room to the SAME DO,
//! which gives us single-writer presence state + naturally-sticky
//! WebSocket fan-out without any coordination on our side.
//!
//! State that lives inside each DO:
//! - `members: HashMap<user_id, presence_json>` — who's in the
//!   room + their latest presence blob.
//! - WebSocket sessions accepted via `state.accept_web_socket()`
//!   so connections survive DO hibernation (Cloudflare bills only
//!   for active CPU; idle WS clients cost nothing).
//!
//! State persisted to DO storage:
//! - `members:<user_id>` keys so a DO restart restores presence
//!   from before the eviction.
//!
//! # The sync/async gap
//!
//! `RoomOps` is a sync trait (shared with the self-hosted runtime
//! that uses an in-process Mutex). DO communication is async over
//! `Stub::fetch_with_request`. The adapter uses
//! `futures::executor::block_on` to bridge — same pattern KvCache,
//! R2Files, and the email transport use. Workers' single-threaded
//! runtime tolerates this inside request handlers.
//!
//! # JS shim
//!
//! `#[durable_object]` generates the JS class export. The
//! `worker-build` step picks it up from this file automatically as
//! long as the crate is compiled with `--features workers` on
//! wasm32. Wrangler binding is declared in `wrangler.toml` under
//! `[[durable_objects.bindings]]`.

#![cfg(feature = "workers")]

use std::cell::RefCell;
use std::collections::HashMap;

use pylon_http::DataError;
use pylon_router::RoomOps;
// Note: import `Result` as the worker alias (not aliased) so the
// async_trait expansion of the DurableObject trait matches our
// impl signatures exactly. Aliasing as `Result` confused the
// async_trait lifetime machinery and produced "lifetime
// parameters or bounds do not match" errors.
use worker::{
    durable_object, DurableObject, Env, Method, Request, RequestInit, Response, Result, State,
    WebSocketPair,
};

// ---------------------------------------------------------------------------
// PylonRoom — Durable Object class
// ---------------------------------------------------------------------------

/// One DO instance per room name. Holds the membership map +
/// hibernation-friendly WebSocket sessions for that room.
///
/// Routing protocol (internal HTTP between the adapter and this
/// DO): every RoomOps method maps to a POST against a specific
/// path on the DO's stub:
///
/// - `POST /join`        body: `{"user_id":..., "data":...}`     → `{"state":..., "presence":...}`
/// - `POST /leave`       body: `{"user_id":...}`                  → `{"presence":...}`
/// - `POST /presence`    body: `{"user_id":..., "data":...}`     → `{"presence":...}`
/// - `POST /broadcast`   body: `{"sender":..., "topic":..., "data":...}` → `{"delivered":N}`
/// - `GET  /members`                                              → `[<presence>, ...]`
/// - `GET  /size`                                                 → `{"size":N}`
/// - `GET  /ws`                                                   → 101 WebSocket upgrade
#[durable_object]
pub struct PylonRoom {
    state: State,
    env: Env,
    /// In-memory presence map. Mirrors what's in storage; rebuilt
    /// from storage on first request after a hibernation wake.
    members: RefCell<HashMap<String, serde_json::Value>>,
}

// The `#[durable_object]` attribute on this impl block generates
// the wasm-bindgen JS shim + per-method Promise wrappers; the
// marker trait declared via `#[durable_object]` on the struct
// above tells the compiler this trait impl must be present.
#[durable_object]
impl DurableObject for PylonRoom {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            members: RefCell::new(HashMap::new()),
        }
    }

    async fn fetch(&mut self, req: Request) -> Result<Response> {
        let mut req = req;
        // Lazy-load membership from storage on first request after
        // wake. `storage.list_with_options(prefix="members:")` returns
        // every persisted member; rebuild the in-memory map.
        if self.members.borrow().is_empty() {
            self.hydrate_from_storage().await;
        }

        let path = req.path();
        let method = req.method();
        match (method, path.as_str()) {
            (Method::Post, "/join") => self.handle_join(&mut req).await,
            (Method::Post, "/leave") => self.handle_leave(&mut req).await,
            (Method::Post, "/presence") => self.handle_presence(&mut req).await,
            (Method::Post, "/broadcast") => self.handle_broadcast(&mut req).await,
            (Method::Get, "/members") => self.handle_members().await,
            (Method::Get, "/size") => self.handle_size().await,
            (Method::Get, "/ws") => self.handle_websocket_upgrade().await,
            _ => Response::error("not found", 404),
        }
    }

    /// WebSocket message handler — wired via `accept_web_socket`
    /// at upgrade time. Browser clients send JSON `{topic, data}`
    /// frames that get fanned out to everyone else in the room.
    async fn websocket_message(
        &mut self,
        ws: worker::WebSocket,
        msg: worker::WebSocketIncomingMessage,
    ) -> Result<()> {
        let text = match msg {
            worker::WebSocketIncomingMessage::String(s) => s,
            worker::WebSocketIncomingMessage::Binary(_) => return Ok(()),
        };
        // Fan out to every connected WS. Worker's WebSocket
        // doesn't expose stable peer IDs, so we send to all
        // including the sender — sender-dedup is the caller's
        // responsibility (clients usually filter on their own
        // message id). The `ws` parameter is unused intentionally
        // (the sender's frame is included via get_websockets()).
        let _ = &ws;
        for peer in self.state.get_websockets() {
            let _ = peer.send_with_str(&text);
        }
        Ok(())
    }

    async fn websocket_close(
        &mut self,
        _ws: worker::WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        // No-op; presence cleanup happens via /leave from the
        // adapter side. WS clients that disconnect without leaving
        // remain in the member list until a future /leave or until
        // the DO evicts them via TTL (not implemented yet).
        Ok(())
    }
}

impl PylonRoom {
    async fn hydrate_from_storage(&self) {
        let storage = self.state.storage();
        let Ok(map) = storage.list().await else {
            return;
        };
        // js_sys::Map iterates via for_each(callback). Use a
        // RefCell to collect pairs out of the callback's
        // by-value closure environment.
        let collected: std::cell::RefCell<Vec<(String, serde_json::Value)>> =
            std::cell::RefCell::new(Vec::new());
        map.for_each(&mut |value, key| {
            let Some(key_str) = key.as_string() else {
                return;
            };
            let Some(user_id) = key_str.strip_prefix("members:") else {
                return;
            };
            if let Ok(presence) = serde_wasm_bindgen::from_value::<serde_json::Value>(value) {
                collected.borrow_mut().push((user_id.to_string(), presence));
            }
        });
        let mut members = self.members.borrow_mut();
        for (user_id, presence) in collected.into_inner() {
            members.insert(user_id, presence);
        }
    }

    async fn handle_join(&self, req: &mut Request) -> Result<Response> {
        #[derive(serde::Deserialize)]
        struct Body {
            user_id: String,
            data: Option<serde_json::Value>,
        }
        let body: Body = req.json().await?;
        let presence = body.data.unwrap_or(serde_json::json!({}));
        self.members
            .borrow_mut()
            .insert(body.user_id.clone(), presence.clone());
        let _ = self
            .state
            .storage()
            .put(&format!("members:{}", body.user_id), presence.clone())
            .await;
        let state_blob = serde_json::json!({
            "members": self.members.borrow().clone(),
        });
        Response::from_json(&serde_json::json!({
            "state": state_blob,
            "presence": presence,
        }))
    }

    async fn handle_leave(&self, req: &mut Request) -> Result<Response> {
        #[derive(serde::Deserialize)]
        struct Body {
            user_id: String,
        }
        let body: Body = req.json().await?;
        let removed = self.members.borrow_mut().remove(&body.user_id);
        let _ = self
            .state
            .storage()
            .delete(&format!("members:{}", body.user_id))
            .await;
        Response::from_json(&serde_json::json!({ "presence": removed }))
    }

    async fn handle_presence(&self, req: &mut Request) -> Result<Response> {
        #[derive(serde::Deserialize)]
        struct Body {
            user_id: String,
            data: serde_json::Value,
        }
        let body: Body = req.json().await?;
        // Match the native RoomManager::set_presence contract:
        // returns None when the user hasn't joined. Updating
        // presence must NOT silently create membership — that
        // would let `/api/rooms/presence` bypass the
        // join-event/snapshot path and produce divergent
        // semantics between native + Workers backends.
        if !self.members.borrow().contains_key(&body.user_id) {
            return Response::from_json(&serde_json::json!({ "presence": null }));
        }
        self.members
            .borrow_mut()
            .insert(body.user_id.clone(), body.data.clone());
        let _ = self
            .state
            .storage()
            .put(&format!("members:{}", body.user_id), body.data.clone())
            .await;
        Response::from_json(&serde_json::json!({ "presence": body.data }))
    }

    async fn handle_broadcast(&self, req: &mut Request) -> Result<Response> {
        #[derive(serde::Deserialize)]
        struct Body {
            sender: Option<String>,
            topic: String,
            data: serde_json::Value,
        }
        let body: Body = req.json().await?;
        let frame = serde_json::json!({
            "type": "broadcast",
            "sender": body.sender,
            "topic": body.topic,
            "data": body.data,
        })
        .to_string();
        let mut delivered = 0;
        for ws in self.state.get_websockets() {
            if ws.send_with_str(&frame).is_ok() {
                delivered += 1;
            }
        }
        Response::from_json(&serde_json::json!({ "delivered": delivered }))
    }

    async fn handle_members(&self) -> Result<Response> {
        let entries: Vec<serde_json::Value> = self.members.borrow().values().cloned().collect();
        Response::from_json(&entries)
    }

    async fn handle_size(&self) -> Result<Response> {
        Response::from_json(&serde_json::json!({ "size": self.members.borrow().len() }))
    }

    async fn handle_websocket_upgrade(&self) -> Result<Response> {
        let pair = WebSocketPair::new()?;
        // Hibernation-compatible — DO can sleep + reawake without
        // losing the connection.
        self.state.accept_web_socket(&pair.server);
        Response::from_websocket(pair.client)
    }
}

// ---------------------------------------------------------------------------
// WorkersRooms — RoomOps adapter
// ---------------------------------------------------------------------------

/// Adapter that routes each RoomOps call to the matching DO
/// instance. Owns the `Env` binding for `PYLON_ROOMS`; resolves
/// `room_name` → stub via `id_from_name`, then makes an internal
/// HTTP request against the stub.
///
/// `Env` is `!Send` on wasm32 (it wraps `JsValue`), but Workers'
/// single-threaded runtime makes Send + Sync requirements moot.
/// SAFETY: the trait requires Send + Sync, which we satisfy via
/// unsafe impls because wasm32 has no real threads — every
/// pointer crossing a "thread boundary" stays on the same OS
/// thread in practice. Same workaround KvCache + R2Files use.
pub struct WorkersRooms {
    env: Env,
    binding: String,
}

// SAFETY: Workers runs on a single thread per request handler.
// JsValue isn't really thread-safe, but the framework's Send +
// Sync bounds never get exercised across threads on wasm32.
unsafe impl Send for WorkersRooms {}
unsafe impl Sync for WorkersRooms {}

impl WorkersRooms {
    pub fn new(env: Env, binding: impl Into<String>) -> Self {
        Self {
            env,
            binding: binding.into(),
        }
    }

    fn do_request(
        &self,
        room: &str,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> std::result::Result<serde_json::Value, String> {
        let ns = self
            .env
            .durable_object(&self.binding)
            .map_err(|e| format!("durable_object binding {}: {e}", self.binding))?;
        let id = ns
            .id_from_name(room)
            .map_err(|e| format!("id_from_name({room}): {e}"))?;
        let stub = id.get_stub().map_err(|e| format!("get_stub: {e}"))?;
        let url = format!("https://do.invalid{path}");
        let mut init = RequestInit::new();
        init.with_method(method);
        if let Some(b) = body {
            init.with_body(Some(b.to_string().into()));
        }
        let req = Request::new_with_init(&url, &init).map_err(|e| e.to_string())?;
        let mut resp = futures::executor::block_on(stub.fetch_with_request(req))
            .map_err(|e| format!("fetch_with_request: {e}"))?;
        let text =
            futures::executor::block_on(resp.text()).map_err(|e| format!("read response: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("parse json: {e} ({text})"))
    }
}

impl RoomOps for WorkersRooms {
    fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> std::result::Result<(serde_json::Value, serde_json::Value), DataError> {
        let body = serde_json::json!({ "user_id": user_id, "data": data });
        let out = self
            .do_request(room, Method::Post, "/join", Some(body))
            .map_err(|e| DataError {
                code: "ROOM_JOIN_FAILED".into(),
                message: e,
            })?;
        let state = out.get("state").cloned().unwrap_or(serde_json::json!({}));
        let presence = out
            .get("presence")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        Ok((state, presence))
    }

    fn leave(&self, room: &str, user_id: &str) -> Option<serde_json::Value> {
        let body = serde_json::json!({ "user_id": user_id });
        self.do_request(room, Method::Post, "/leave", Some(body))
            .ok()
            .and_then(|v| v.get("presence").cloned())
    }

    fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let body = serde_json::json!({ "user_id": user_id, "data": data });
        self.do_request(room, Method::Post, "/presence", Some(body))
            .ok()
            .and_then(|v| v.get("presence").cloned())
    }

    fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let body = serde_json::json!({
            "sender": sender,
            "topic": topic,
            "data": data,
        });
        self.do_request(room, Method::Post, "/broadcast", Some(body))
            .ok()
    }

    fn list_rooms(&self) -> Vec<String> {
        // DO doesn't expose a room enumeration API — there's no
        // namespace-wide listing. Returning empty is the honest
        // answer; callers that need this would maintain a parallel
        // KV-side index of active room names. Workers RoomOps
        // documents this as a limit of the DO model.
        vec![]
    }

    fn room_size(&self, name: &str) -> usize {
        self.do_request(name, Method::Get, "/size", None)
            .ok()
            .and_then(|v| v.get("size").and_then(|s| s.as_u64()).map(|n| n as usize))
            .unwrap_or(0)
    }

    fn members(&self, name: &str) -> Vec<serde_json::Value> {
        self.do_request(name, Method::Get, "/members", None)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
    }
}
