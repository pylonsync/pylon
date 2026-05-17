//! `PylonPubSub` Durable Object class + `WorkersPubSub`
//! `PubSubOps` adapter.
//!
//! Pub/Sub on Workers maps one DO per channel name (same routing
//! trick PylonRoom uses). Each DO holds:
//! - in-memory list of accepted WebSocket subscribers
//! - last-N message history persisted to DO storage so reconnecting
//!   clients can catch up
//!
//! The PubSubOps trait is router-handler-shaped (handle_publish
//! takes a JSON body string, returns status+body), so the adapter
//! parses out the channel name + payload and forwards to the
//! channel's DO via internal HTTP.

#![cfg(feature = "workers")]

use std::cell::RefCell;
use std::collections::VecDeque;

use pylon_router::PubSubOps;
use worker::{
    durable_object, DurableObject, Env, Method, Request, RequestInit, Response, Result, State,
    WebSocketPair,
};

const HISTORY_CAP: usize = 100;

#[durable_object]
pub struct PylonPubSub {
    state: State,
    env: Env,
    /// Rolling last-HISTORY_CAP messages for catch-up on
    /// reconnect. Hydrated from storage on first request after
    /// hibernation wake.
    history: RefCell<VecDeque<serde_json::Value>>,
    /// Monotonic publish sequence — used as the storage key
    /// suffix so older entries don't get overwritten when the
    /// in-memory ring wraps. Persisted to `seq` so it survives
    /// DO hibernation; rebuilt from storage on first request.
    next_seq: RefCell<usize>,
}

#[durable_object]
impl DurableObject for PylonPubSub {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            history: RefCell::new(VecDeque::with_capacity(HISTORY_CAP)),
            next_seq: RefCell::new(0),
        }
    }

    async fn fetch(&mut self, req: Request) -> Result<Response> {
        let mut req = req;
        if self.history.borrow().is_empty() {
            self.hydrate().await;
        }
        let path = req.path();
        let method = req.method();
        match (method, path.as_str()) {
            (Method::Post, "/publish") => self.handle_publish(&mut req).await,
            (Method::Get, "/history") => self.handle_history().await,
            (Method::Get, "/subscribe") => self.handle_subscribe().await,
            _ => Response::error("not found", 404),
        }
    }
}

impl PylonPubSub {
    async fn hydrate(&self) {
        let storage = self.state.storage();
        let Ok(map) = storage.list().await else {
            return;
        };
        let collected: std::cell::RefCell<Vec<(usize, serde_json::Value)>> =
            std::cell::RefCell::new(Vec::new());
        let mut max_seq = 0usize;
        map.for_each(&mut |value, key| {
            let Some(key_str) = key.as_string() else {
                return;
            };
            // Keys are "h:<seq>"; sort by seq to restore order.
            let Some(seq_str) = key_str.strip_prefix("h:") else {
                return;
            };
            let Ok(seq) = seq_str.parse::<usize>() else {
                return;
            };
            if seq + 1 > max_seq {
                max_seq = seq + 1;
            }
            if let Ok(msg) = serde_wasm_bindgen::from_value::<serde_json::Value>(value) {
                collected.borrow_mut().push((seq, msg));
            }
        });
        let mut entries = collected.into_inner();
        entries.sort_by_key(|(s, _)| *s);
        let mut hist = self.history.borrow_mut();
        for (_, msg) in entries.into_iter().rev().take(HISTORY_CAP).rev() {
            hist.push_back(msg);
        }
        // Resume the seq counter so subsequent publishes don't
        // overwrite stored entries.
        *self.next_seq.borrow_mut() = max_seq;
    }

    async fn handle_publish(&self, req: &mut Request) -> Result<Response> {
        let payload: serde_json::Value = req.json().await?;
        // Monotonic seq for the storage key so we don't overwrite
        // stored entries when the in-memory ring wraps. Earlier
        // version used hist.len() which caps at HISTORY_CAP after
        // pop_front — every later publish would clobber the last
        // slot (P2 finding from codex).
        let seq = {
            let mut n = self.next_seq.borrow_mut();
            let v = *n;
            *n = n.saturating_add(1);
            v
        };
        let mut hist = self.history.borrow_mut();
        if hist.len() >= HISTORY_CAP {
            hist.pop_front();
        }
        hist.push_back(payload.clone());
        drop(hist);
        let _ = self
            .state
            .storage()
            .put(&format!("h:{seq}"), payload.clone())
            .await;
        // Evict the oldest persisted entry too so storage stays
        // bounded — without this, the KV under the DO grows
        // unbounded over the channel's lifetime.
        if seq >= HISTORY_CAP {
            let _ = self
                .state
                .storage()
                .delete(&format!("h:{}", seq - HISTORY_CAP))
                .await;
        }
        // Fan out to subscribers. Workers WebSocket doesn't give
        // us a stable subscriber id — we send to everyone, the
        // client filters by their own subscription state.
        let frame = serde_json::to_string(&payload).unwrap_or_default();
        let mut delivered = 0;
        for ws in self.state.get_websockets() {
            if ws.send_with_str(&frame).is_ok() {
                delivered += 1;
            }
        }
        Response::from_json(&serde_json::json!({ "delivered": delivered }))
    }

    async fn handle_history(&self) -> Result<Response> {
        let entries: Vec<serde_json::Value> = self.history.borrow().iter().cloned().collect();
        Response::from_json(&entries)
    }

    async fn handle_subscribe(&self) -> Result<Response> {
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        Response::from_websocket(pair.client)
    }
}

// ---------------------------------------------------------------------------
// WorkersPubSub — PubSubOps adapter
// ---------------------------------------------------------------------------

pub struct WorkersPubSub {
    env: Env,
    binding: String,
}

unsafe impl Send for WorkersPubSub {}
unsafe impl Sync for WorkersPubSub {}

impl WorkersPubSub {
    pub fn new(env: Env, binding: impl Into<String>) -> Self {
        Self {
            env,
            binding: binding.into(),
        }
    }

    fn do_request(
        &self,
        channel: &str,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> std::result::Result<(u16, String), String> {
        let ns = self
            .env
            .durable_object(&self.binding)
            .map_err(|e| format!("durable_object binding {}: {e}", self.binding))?;
        let id = ns
            .id_from_name(channel)
            .map_err(|e| format!("id_from_name({channel}): {e}"))?;
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
        let status = resp.status_code();
        let text =
            futures::executor::block_on(resp.text()).map_err(|e| format!("read response: {e}"))?;
        Ok((status, text))
    }
}

impl PubSubOps for WorkersPubSub {
    fn handle_publish(&self, body: &str) -> (u16, String) {
        // Body shape: {"channel": "...", "data": {...}}. The
        // handler parses channel out before routing.
        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    pylon_router::json_error("INVALID_JSON", &format!("invalid body: {e}")),
                )
            }
        };
        let channel = parsed.get("channel").and_then(|c| c.as_str()).unwrap_or("");
        if channel.is_empty() {
            return (
                400,
                pylon_router::json_error("MISSING_CHANNEL", "body needs `channel`"),
            );
        }
        // Accept either `message` (the existing /api/pubsub/publish
        // contract — what every framework client sends) or `data`
        // (a Workers-only shorthand). Default to the parsed body
        // minus `channel` if neither key is present, so structured
        // payloads still work.
        let data = parsed
            .get("message")
            .cloned()
            .or_else(|| parsed.get("data").cloned())
            .unwrap_or_else(|| {
                let mut copy = parsed.clone();
                if let Some(obj) = copy.as_object_mut() {
                    obj.remove("channel");
                }
                copy
            });
        match self.do_request(channel, Method::Post, "/publish", Some(data)) {
            Ok((status, body)) => (status, body),
            Err(e) => (502, pylon_router::json_error("PUBSUB_DO_FAILED", &e)),
        }
    }

    fn handle_channels(&self) -> (u16, String) {
        // DO has no namespace-listing API. Customers wanting this
        // maintain a parallel KV index, same as Rooms. Empty list is
        // the honest answer.
        (200, "[]".into())
    }

    fn handle_history(&self, channel: &str, _url: &str) -> (u16, String) {
        match self.do_request(channel, Method::Get, "/history", None) {
            Ok((status, body)) => (status, body),
            Err(e) => (502, pylon_router::json_error("PUBSUB_DO_FAILED", &e)),
        }
    }
}
