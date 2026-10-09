//! `ctx.llm.stream` through the real Bun runtime (`packages/functions`).
//!
//! The provider is a stub `LlmStreamHook` that emits tokens with delays.
//! The function forwards each token to `ctx.stream.write`, as the agent
//! loop does. The tests check what the client (`on_stream`) sees and
//! when:
//!
//!   - tokens reach the client while the provider is still generating,
//!     not all at once after it returns
//!   - a call that ends while its stream is still running cancels the
//!     provider request
//!   - aborting the stream's signal cancels the provider request while
//!     the call keeps running
//!   - a provider that is silent for longer than the idle timeout does
//!     not time the call out while it is still running
//!
//! Skipped (with a message) when `bun` is not on PATH.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pylon_functions::protocol::{AuthInfo, FnType};
use pylon_functions::runner::{FnRunner, StreamChunk};
use pylon_http::{DataError, DataStore};

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn runtime_ts() -> PathBuf {
    without_verbatim_prefix(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/functions/src/runtime.ts")
            .canonicalize()
            .expect("packages/functions/src/runtime.ts"),
    )
}

/// `canonicalize` returns a `\\?\` verbatim path on Windows, which bun
/// cannot resolve as a module path. Drop the prefix; other platforms are
/// unchanged.
fn without_verbatim_prefix(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// A temp app dir with `functions/<name>.ts` for each entry.
fn app_dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pylon-llm-stream-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("functions")).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join("functions").join(format!("{name}.ts")), src).unwrap();
    }
    dir
}

fn start_runner(dir: &Path) -> FnRunner {
    let runner = FnRunner::new(16);
    let script = format!(
        "cd '{}' && exec bun '{}' functions",
        dir.display(),
        runtime_ts().display()
    );
    runner
        .start("sh", &["-c", &script])
        .expect("bun runtime starts");
    runner
}

fn auth() -> AuthInfo {
    AuthInfo {
        user_id: Some("u1".into()),
        is_admin: false,
        tenant_id: None,
        roles: vec![],
        is_guest: false,
    }
}

fn response(text: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "stub",
        "content": [{ "type": "text", "text": text }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1, "output_tokens": 1 },
    })
}

const FORWARDING_FN: &str = r#"
export default {
  type: "action",
  handler: async (ctx) => {
    const res = await ctx.llm.stream({ messages: [{ role: "user", content: "hi" }] }, (e) => {
      if (e.type === "text_delta") ctx.stream.write(e.text);
    });
    return { stop: res.stop_reason };
  },
};
"#;

#[test]
fn stream_frames_reach_the_client_while_the_provider_generates() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = app_dir("live", &[("streamer", FORWARDING_FN)]);
    let runner = start_runner(&dir);

    let provider_done: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let done_for_hook = Arc::clone(&provider_done);
    runner.set_llm_stream_hook(Box::new(move |_req, _auth, on_event, _cancel| {
        for token in ["a", "b", "c"] {
            on_event(serde_json::json!({ "type": "text_delta", "text": token }));
            std::thread::sleep(Duration::from_millis(300));
        }
        *done_for_hook.lock().unwrap() = Some(Instant::now());
        Ok(response("abc"))
    }));

    let received: Arc<Mutex<Vec<(String, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let (value, _) = runner
        .call(
            &NullStore,
            "streamer",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            Some(Box::new(move |chunk: StreamChunk<'_>| {
                sink.lock()
                    .unwrap()
                    .push((chunk.data.to_string(), Instant::now()));
            })),
            None,
            None,
        )
        .expect("call succeeds");
    assert_eq!(value["stop"], "end_turn");

    let received = received.lock().unwrap().clone();
    let tokens: Vec<&str> = received.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(tokens, ["a", "b", "c"]);
    let done = provider_done.lock().unwrap().expect("provider finished");
    // The first token was written ~600ms before the provider returned.
    // Delivered live, it arrives well before that; buffered behind the
    // provider call, it arrives after.
    let first = received[0].1;
    assert!(
        first + Duration::from_millis(300) < done,
        "the first token reached the client {:?} after the provider finished",
        first.saturating_duration_since(done)
    );
    runner.kill();
}

#[test]
fn a_call_that_ends_cancels_its_running_stream() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    // Starts a stream and returns without awaiting it.
    let dir = app_dir(
        "detached",
        &[(
            "detached",
            r#"
export default {
  type: "action",
  handler: async (ctx) => {
    ctx.llm.stream({ messages: [] }, () => {}).catch(() => {});
    await new Promise((r) => setTimeout(r, 200));
    return { ok: true };
  },
};
"#,
        )],
    );
    let runner = start_runner(&dir);

    let emitted = Arc::new(AtomicUsize::new(0));
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let (emitted_h, saw_cancel_h) = (Arc::clone(&emitted), Arc::clone(&saw_cancel));
    runner.set_llm_stream_hook(Box::new(move |_req, _auth, on_event, cancel| {
        for _ in 0..100 {
            if cancel.load(Ordering::SeqCst) {
                saw_cancel_h.store(true, Ordering::SeqCst);
                return Err(("LLM_CANCELLED".into(), "cancelled".into()));
            }
            on_event(serde_json::json!({ "type": "text_delta", "text": "x" }));
            emitted_h.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(response("x"))
    }));

    let started = Instant::now();
    let (value, _) = runner
        .call(
            &NullStore,
            "detached",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            None,
            None,
            None,
        )
        .expect("call succeeds");
    assert_eq!(value["ok"], true);
    // The call returns on its own schedule, not after the 5s stream.
    assert!(started.elapsed() < Duration::from_secs(3));

    let deadline = Instant::now() + Duration::from_secs(3);
    while !saw_cancel.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        saw_cancel.load(Ordering::SeqCst),
        "the provider call was not cancelled when the call ended"
    );
    assert!(emitted.load(Ordering::SeqCst) < 100);
    runner.kill();
}

#[test]
fn aborting_the_signal_cancels_the_provider_call_mid_stream() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    // Aborts after the second token, then keeps running for a second so
    // the cancel cannot come from the call ending.
    let dir = app_dir(
        "abort",
        &[(
            "stopper",
            r#"
export default {
  type: "action",
  handler: async (ctx) => {
    const controller = new AbortController();
    let seen = 0;
    const code = await ctx.llm
      .stream({ messages: [] }, () => {
        seen += 1;
        if (seen === 2) controller.abort();
      }, { signal: controller.signal })
      .then(() => "resolved", (e) => e.code);
    await new Promise((r) => setTimeout(r, 1000));
    return { code, seen };
  },
};
"#,
        )],
    );
    let runner = start_runner(&dir);

    let cancelled_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let cancelled_h = Arc::clone(&cancelled_at);
    runner.set_llm_stream_hook(Box::new(move |_req, _auth, on_event, cancel| {
        for _ in 0..100 {
            if cancel.load(Ordering::SeqCst) {
                *cancelled_h.lock().unwrap() = Some(Instant::now());
                return Err(("LLM_CANCELLED".into(), "cancelled".into()));
            }
            on_event(serde_json::json!({ "type": "text_delta", "text": "x" }));
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(response("x"))
    }));

    let (value, _) = runner
        .call(
            &NullStore,
            "stopper",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            None,
            None,
            None,
        )
        .expect("call succeeds");
    let returned = Instant::now();
    assert_eq!(value["code"], "LLM_CANCELLED");
    let cancelled = cancelled_at
        .lock()
        .unwrap()
        .expect("the provider call saw the cancel");
    assert!(
        cancelled + Duration::from_millis(500) < returned,
        "the cancel arrived only when the call ended"
    );
    runner.kill();
}

#[test]
fn a_silent_provider_does_not_trip_the_idle_timeout() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = app_dir("silent", &[("streamer", FORWARDING_FN)]);
    let runner = start_runner(&dir);
    runner.set_call_timeout(Duration::from_millis(700));
    runner.set_llm_stream_hook(Box::new(|_req, _auth, on_event, _cancel| {
        // Thinking: nothing for longer than the idle timeout.
        std::thread::sleep(Duration::from_millis(1500));
        on_event(serde_json::json!({ "type": "text_delta", "text": "done" }));
        Ok(response("done"))
    }));
    let (value, _) = runner
        .call(
            &NullStore,
            "streamer",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            None,
            None,
            None,
        )
        .expect("a running provider keeps the call alive");
    assert_eq!(value["stop"], "end_turn");
    runner.kill();
}

/// The handler keeps working after the provider returns. The provider's
/// finish is activity: the idle budget starts over from it, so the call
/// does not time out while the handler finishes up.
#[test]
fn a_handler_that_resumes_after_the_provider_gets_a_fresh_idle_budget() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    const RESUMING_FN: &str = r#"
export default {
  type: "action",
  handler: async (ctx) => {
    const res = await ctx.llm.stream({ messages: [{ role: "user", content: "hi" }] }, () => {});
    // Work after the provider answered, well within one idle timeout.
    await new Promise((r) => setTimeout(r, 450));
    return { stop: res.stop_reason };
  },
};
"#;
    let dir = app_dir("resume", &[("resumer", RESUMING_FN)]);
    let runner = start_runner(&dir);
    runner.set_call_timeout(Duration::from_millis(700));
    // Finishes just before the extended deadline the running provider
    // earned (700 ms after the first timeout at 700 ms).
    runner.set_llm_stream_hook(Box::new(|_req, _auth, _on_event, _cancel| {
        std::thread::sleep(Duration::from_millis(1250));
        Ok(response("done"))
    }));
    let (value, _) = runner
        .call(
            &NullStore,
            "resumer",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            None,
            None,
            None,
        )
        .expect("the provider's finish restarts the idle budget");
    assert_eq!(value["stop"], "end_turn");
    runner.kill();
}

/// The functions under test never touch `ctx.db`.
struct NullStore;

impl DataStore for NullStore {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        static M: std::sync::OnceLock<pylon_kernel::AppManifest> = std::sync::OnceLock::new();
        M.get_or_init(pylon_kernel::AppManifest::default)
    }
    fn insert(&self, _: &str, _: &serde_json::Value) -> Result<String, DataError> {
        unreachable!()
    }
    fn get_by_id(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, DataError> {
        Ok(None)
    }
    fn list(&self, _: &str) -> Result<Vec<serde_json::Value>, DataError> {
        Ok(vec![])
    }
    fn list_after(
        &self,
        _: &str,
        _: Option<&str>,
        _: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Ok(vec![])
    }
    fn update(&self, _: &str, _: &str, _: &serde_json::Value) -> Result<bool, DataError> {
        unreachable!()
    }
    fn delete(&self, _: &str, _: &str) -> Result<bool, DataError> {
        unreachable!()
    }
    fn lookup(&self, _: &str, _: &str, _: &str) -> Result<Option<serde_json::Value>, DataError> {
        Ok(None)
    }
    fn link(&self, _: &str, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
        unreachable!()
    }
    fn unlink(&self, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
        unreachable!()
    }
    fn query_filtered(
        &self,
        _: &str,
        _: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Ok(vec![])
    }
    fn query_graph(&self, _: &serde_json::Value) -> Result<serde_json::Value, DataError> {
        Ok(serde_json::json!({}))
    }
    fn transact(
        &self,
        _: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        unreachable!()
    }
}

#[test]
fn provider_calls_on_one_runner_overlap_and_keep_auth_separate() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = app_dir(
        "provider-concurrency",
        &[(
            "provider",
            r#"
export default {
  type: "action",
  handler: async (ctx, args) => {
    if (args.kind === "complete") return await ctx.llm.complete({ messages: [] });
    if (args.kind === "embed") return await ctx.llm.embed([ctx.auth.userId]);
    if (args.kind === "files") return await ctx.files.store("bytes", { name: ctx.auth.userId });
    if (args.kind === "connections") return (await ctx.connections.get("test")).accessToken;
    await ctx.email.send(ctx.auth.userId, "test", "test");
    return ctx.auth.userId;
  },
};
"#,
        )],
    );
    let runner = start_runner(&dir);
    for kind in ["complete", "embed", "email", "files", "connections"] {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let release_hook = Arc::clone(&release);
        let enter = move |user: &str| {
            entered_tx.send(user.to_owned()).unwrap();
            let (lock, ready) = &*release_hook;
            let (released, timeout) = ready
                .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(10), |v| !*v)
                .unwrap();
            assert!(
                *released && !timeout.timed_out(),
                "test did not release provider"
            );
        };
        match kind {
            "complete" => runner.set_llm_hook(Box::new(move |_, auth| {
                let user = auth.user_id.as_deref().unwrap();
                enter(user);
                Ok(serde_json::json!(user))
            })),
            "embed" => runner.set_llm_embed_hook(Box::new(move |request, auth| {
                let user = auth.user_id.as_deref().unwrap();
                assert_eq!(request["input"][0], user);
                enter(user);
                Ok(serde_json::json!({ "embeddings": [[if user == "u1" { 1 } else { 2 }]] }))
            })),
            "files" => runner.set_files_op_hook(Box::new(move |op, auth| {
                let user = auth.user_id.as_deref().unwrap();
                let pylon_functions::protocol::FilesOp::Store { name, .. } = op else {
                    panic!("expected a file write");
                };
                assert_eq!(name, user);
                enter(user);
                Ok(serde_json::json!(user))
            })),
            "connections" => runner.set_connection_hook(Box::new(move |op, payload, auth| {
                assert_eq!(op, "get");
                assert_eq!(payload["name"], "test");
                let user = auth.user_id.as_deref().unwrap();
                enter(user);
                Ok(serde_json::json!({ "access_token": user, "scope": null, "expires_at": null }))
            })),
            _ => runner.set_email_hook(Box::new(move |message| {
                enter(&message.to);
                Ok(())
            })),
        }
        std::thread::scope(|scope| {
            let calls: Vec<_> = ["u1", "u2"]
                .into_iter()
                .map(|user| {
                    let runner = &runner;
                    scope.spawn(move || {
                        let mut caller = auth();
                        caller.user_id = Some(user.into());
                        runner.call(
                            &NullStore,
                            "provider",
                            FnType::Action,
                            serde_json::json!({ "kind": kind }),
                            caller,
                            None,
                            None,
                            None,
                        )
                    })
                })
                .collect();
            let first = entered_rx.recv_timeout(Duration::from_secs(5));
            let second = entered_rx.recv_timeout(Duration::from_secs(5));
            // Always release both calls before asserting, including on failure.
            *release.0.lock().unwrap() = true;
            release.1.notify_all();
            let results: Vec<_> = calls
                .into_iter()
                .map(|call| call.join().unwrap().unwrap().0)
                .collect();
            let mut users = vec![
                first.expect("first provider entered"),
                second.expect("second provider entered before release"),
            ];
            users.sort();
            assert_eq!(users, ["u1", "u2"]);
            if kind == "embed" {
                assert_eq!(
                    results,
                    [serde_json::json!([[1]]), serde_json::json!([[2]])]
                );
            } else {
                assert_eq!(results, [serde_json::json!("u1"), serde_json::json!("u2")]);
            }
        });
    }
    drop(runner);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn trace_error_limits_preserve_the_callers_error() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = app_dir(
        "large-error",
        &[(
            "fails",
            r#"
export default {
  type: "action",
  handler: async (ctx) => { throw ctx.error("TEST_ERROR", "文😀".repeat(10000)); },
};
"#,
        )],
    );
    let runner = start_runner(&dir);
    let error = runner
        .call(
            &NullStore,
            "fails",
            FnType::Action,
            serde_json::json!({}),
            auth(),
            None,
            None,
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "TEST_ERROR");
    assert_eq!(error.message, "文😀".repeat(10000));
    let traces = runner.trace_log.recent(1);
    assert_eq!(traces.len(), 1);
    let saved = serde_json::to_value(&traces[0]).unwrap();
    assert_eq!(saved["outcome"]["code"], "TEST_ERROR");
    let message = saved["outcome"]["message"].as_str().unwrap();
    assert!(message.len() <= 4096);
    assert!(message.ends_with(" [truncated]"));
    drop(runner);
    std::fs::remove_dir_all(dir).unwrap();
}
