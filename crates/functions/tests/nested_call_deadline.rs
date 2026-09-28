//! A nested call (`ctx.runQuery`, `ctx.agents.run`) that takes longer than
//! the caller's idle timeout does not time the caller out once it
//! returns. The nested call runs on the caller's thread, so the caller's
//! idle budget is renewed when it comes back.
//!
//! Drives the real Bun runtime (`packages/functions`). Skipped (with a
//! message) when `bun` is not on PATH.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pylon_functions::protocol::{AuthInfo, FnType};
use pylon_functions::runner::FnRunner;
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

#[test]
fn a_slow_nested_call_does_not_time_out_its_caller() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "pylon-nested-deadline-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("functions")).unwrap();
    // The nested query declares its own 10s timeout and takes 2.5s; the
    // caller has the global 1s idle timeout.
    std::fs::write(
        dir.join("functions/slow.ts"),
        r#"export default {
  type: "query",
  timeout: 10,
  handler: async () => {
    await new Promise((r) => setTimeout(r, 2500));
    return "slow done";
  },
};
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("functions/caller.ts"),
        r#"export default {
  type: "action",
  handler: async (ctx) => ({ nested: await ctx.runQuery("slow", {}) }),
};
"#,
    )
    .unwrap();

    let runner = FnRunner::new(16);
    let script = format!(
        "cd '{}' && exec bun '{}' functions",
        dir.display(),
        runtime_ts().display()
    );
    runner
        .start("sh", &["-c", &script])
        .expect("bun runtime starts");
    runner.set_call_timeout(Duration::from_secs(1));

    let auth = AuthInfo {
        user_id: Some("u1".into()),
        is_admin: false,
        tenant_id: None,
        roles: vec![],
        is_guest: false,
    };
    let (value, _) = runner
        .call(
            &NullStore,
            "caller",
            FnType::Action,
            serde_json::json!({}),
            auth,
            None,
            None,
            None,
        )
        .expect("the caller outlives a slow nested call");
    assert_eq!(value["nested"], "slow done");
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
