//! `/api/sync/*` + GDPR `/api/admin/users/:id/{export,purge}` routes.
//!
//! Sync pull filters changes through the read-policy fence so callers
//! can't sidestep entity policies via the change feed (regression
//! coverage in the auth-matrix scaffold).
//!
//! Structure: `handle` is a thin top-level router. It dispatches to:
//!   - `handle_snapshot_pull` — snapshot pagination at `since=0`
//!   - `handle_delta_pull`    — change-log tail at `since>0`
//!   - GDPR export/purge      — admin-gated data-subject endpoints
//!   - `handle_push`          — client write path through the
//!                              shared mutation pipeline
//! Plus `project_change_for_caller` — the read-policy + projection
//! filter shared by both pull paths.

use crate::mutate::{apply_mutation, MutationCtx, MutationError, MutationOp};
use crate::{gdpr_export, gdpr_purge, json_error_safe, require_admin, RouterContext};
use pylon_http::HttpMethod;
use pylon_sync::{ChangeEvent, ChangeKind, SyncCursor};

/// Minimal URL percent-decoder for the snapshot cursor query param.
/// Avoids dragging in a `urlencoding` crate for one usage site.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Minimal URL percent-encoder for query params. Encodes anything
/// that isn't an unreserved RFC 3986 character.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Rows per pull page when the client does not ask (`limit=`). Older
/// clients send no limit and keep the page size they were written for.
const DEFAULT_PAGE_ROWS: usize = 1000;

/// The most rows one pull page may carry, whatever the client asks.
pub const MAX_PAGE_ROWS: usize = 10_000;

/// Approximate JSON bytes one pull page may carry. Pages end at the row
/// limit or at this size, whichever comes first, so a client that asks for
/// large pages of wide rows does not get a response of tens of megabytes.
/// `PYLON_SYNC_PAGE_BYTES` overrides the default of 4 MiB.
fn page_byte_budget() -> usize {
    std::env::var("PYLON_SYNC_PAGE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(4 * 1024 * 1024)
}

/// The rows the client asked for per page (`limit=`), clamped to
/// `1..=MAX_PAGE_ROWS`; `DEFAULT_PAGE_ROWS` when absent.
fn requested_page_rows(url: &str) -> usize {
    url.split('?')
        .nth(1)
        .and_then(|qs| {
            qs.split('&')
                .find_map(|kv| kv.strip_prefix("limit="))
                .and_then(|v| v.parse::<usize>().ok())
        })
        .map(|n| n.clamp(1, MAX_PAGE_ROWS))
        .unwrap_or(DEFAULT_PAGE_ROWS)
}

/// Ends a pull page at a row limit or a byte budget.
struct PageBudget {
    rows: usize,
    bytes: usize,
    used: usize,
}

impl PageBudget {
    fn new(rows: usize) -> Self {
        Self::with_bytes(rows, page_byte_budget())
    }

    fn with_bytes(rows: usize, bytes: usize) -> Self {
        Self {
            rows,
            bytes,
            used: 0,
        }
    }

    /// Count one event's approximate wire size.
    fn add(&mut self, ev: &ChangeEvent) {
        let data = ev
            .data
            .as_ref()
            .and_then(|d| serde_json::to_vec(d).ok())
            .map_or(0, |v| v.len());
        // seq, kind, timestamp and the JSON punctuation.
        self.used += data + ev.entity.len() + ev.row_id.len() + ev.timestamp.len() + 64;
    }

    fn full(&self, rows: usize) -> bool {
        rows >= self.rows || self.used >= self.bytes
    }
}

/// Rows a single snapshot request may TOUCH before paginating.
///
/// Overridable without the environment. Tests need a small budget to force
/// pagination, and `std::env::set_var` from a test thread races every other
/// thread reading the environment — which is why it is unsafe, and which
/// showed up as whole test binaries aborting and their remaining tests failing
/// on `connect: ConnectionRefused`. An atomic is the same knob without the UB.
static SNAPSHOT_SCAN_BUDGET: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Override the per-request scanned-row budget. 0 restores the default.
pub fn set_snapshot_scan_budget(rows: usize) {
    SNAPSHOT_SCAN_BUDGET.store(rows, std::sync::atomic::Ordering::Relaxed);
}

fn snapshot_scan_budget() -> usize {
    let overridden = SNAPSHOT_SCAN_BUDGET.load(std::sync::atomic::Ordering::Relaxed);
    if overridden > 0 {
        return overridden;
    }
    std::env::var("PYLON_SNAPSHOT_SCAN_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(50_000)
}

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url.starts_with("/api/sync/pull") && method == HttpMethod::Get {
        return Some(handle_pull(ctx, url));
    }
    if let Some(out) = handle_gdpr(ctx, method, url) {
        return Some(out);
    }
    if url == "/api/sync/push" && method == HttpMethod::Post {
        return Some(handle_push(ctx, body));
    }
    if url.starts_with("/api/sync/relay-token") && method == HttpMethod::Get {
        return Some(handle_relay_token(ctx));
    }
    None
}

// ---------------------------------------------------------------------------
// Relay token
// ---------------------------------------------------------------------------

/// GET /api/sync/relay-token — mint a signed auth blob for the Durable
/// Object sync relay (docs/SYNC_DURABLE_OBJECTS_DESIGN.md).
///
/// The relay filters change events per subscriber with `pylon-policy`
/// but has no session store, so the machine authenticates + enriches
/// ONCE here and signs the resulting context. Anonymous callers get an
/// anonymous blob — same as connecting to the machine's own WS, where
/// policies decide row visibility, not the transport.
///
/// 404 when no relay is configured (opt-in surface, mirrors
/// trusted-mint): plain installs must not expose a signing oracle.
fn handle_relay_token(ctx: &RouterContext) -> (u16, String) {
    let secret = std::env::var("PYLON_SYNC_RELAY_SECRET")
        .ok()
        .filter(|s| !s.is_empty());
    let base = std::env::var("PYLON_SYNC_RELAY_URL")
        .ok()
        .filter(|s| !s.is_empty());
    let (Some(secret), Some(base)) = (secret, base) else {
        return (
            404,
            json_error_safe(
                "NOT_FOUND",
                "No sync relay configured",
                "PYLON_SYNC_RELAY_URL / PYLON_SYNC_RELAY_SECRET are unset",
            ),
        );
    };
    let manifest = ctx.store.manifest();
    let app = std::env::var("PYLON_SYNC_RELAY_APP")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("PYLON_PROJECT_ID")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| manifest.name.clone());
    let ttl = std::env::var("PYLON_SYNC_RELAY_TOKEN_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(pylon_auth::relay_blob::DEFAULT_TTL_SECS);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let exp = now.saturating_add(ttl);
    let token = match pylon_auth::relay_blob::mint(&secret, &app, ctx.auth_ctx, exp) {
        Ok(t) => t,
        Err(e) => {
            return (
                500,
                json_error_safe(
                    "RELAY_MINT_FAILED",
                    "Could not mint relay token",
                    &e.to_string(),
                ),
            );
        }
    };
    // The socket endpoint clients should dial. Override for split
    // ingress; default derives ws(s):// from the relay base URL.
    let ws_url = std::env::var("PYLON_SYNC_RELAY_PUBLIC_WS_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let ws_base = if let Some(rest) = base.strip_prefix("https://") {
                format!("wss://{rest}")
            } else if let Some(rest) = base.strip_prefix("http://") {
                format!("ws://{rest}")
            } else {
                base.clone()
            };
            let mut enc = String::with_capacity(app.len());
            for b in app.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        enc.push(b as char)
                    }
                    other => enc.push_str(&format!("%{other:02X}")),
                }
            }
            format!("{}/sync/ws?app={enc}", ws_base.trim_end_matches('/'))
        });
    (
        200,
        serde_json::json!({ "token": token, "url": ws_url, "exp": exp }).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Pull paths
// ---------------------------------------------------------------------------

fn handle_pull(ctx: &RouterContext, url: &str) -> (u16, String) {
    let since: u64 = url
        .split("since=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if since == 0 {
        handle_snapshot_pull(ctx, url)
    } else {
        handle_delta_pull(ctx, url, since)
    }
}

/// Fresh client (since=0): synthesize a snapshot from current entity
/// rows instead of relying on the in-memory change-log tail. Startup
/// seeds the log with current rows, but retention can evict those
/// seed events before the first client connects; a 50k-row table
/// behind a 10k-event log loses 40k seeds. Snapshot-on-pull closes
/// the gap deterministically: every entity is queried at request
/// time, each row emitted as a synthetic Insert with seq = snapshot
/// as-of seq so the caller's cursor lands at the same position the
/// log tail would have given them.
///
/// Pagination: `?snapshot_after=<urlenc(json)>` carries the entity we
/// paused inside + the last row id emitted for it + the pinned
/// as-of seq, so successive pages share one consistent snapshot
/// frame even if writes land mid-pagination.
fn handle_snapshot_pull(ctx: &RouterContext, url: &str) -> (u16, String) {
    const RAW_FETCH_CHUNK: usize = 200;
    const FILTERED_FETCH_CHUNK: usize = 1000;
    let mut page = PageBudget::new(requested_page_rows(url));

    let snapshot_after_raw: Option<String> = url
        .split("snapshot_after=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .map(|s| s.to_string());
    let snapshot_after_parsed: Option<serde_json::Value> = snapshot_after_raw
        .as_deref()
        .map(url_decode)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    // A present-but-unparseable cursor MUST fail loudly. Falling through to
    // "no cursor" silently restarts the snapshot at page 1, and a client
    // that keeps echoing the bad cursor (e.g. one that re-encoded the
    // opaque value) pulls page 1 in an infinite loop at full tilt.
    if snapshot_after_raw.is_some() && snapshot_after_parsed.is_none() {
        return (
            400,
            json_error_safe(
                "SNAPSHOT_CURSOR_INVALID",
                "snapshot_after did not parse — pass the server-issued value verbatim (it is already URL-encoded)",
                "snapshot_after present but not urlenc(json)",
            ),
        );
    }
    let snapshot_after: Option<(String, Option<String>)> =
        snapshot_after_parsed.as_ref().and_then(|v| {
            let entity = v.get("e").and_then(|s| s.as_str())?.to_string();
            let after_id = v.get("a").and_then(|s| s.as_str()).map(|s| s.to_string());
            Some((entity, after_id))
        });
    // Pin snapshot_seq across all pages of a single client's snapshot
    // fetch. Without that, each page would capture a fresh
    // `current_seq` and a write landing between pages could be
    // skipped (page 1 emitted row at seq=N, write landed at seq=N+1,
    // final page advanced cursor to N+2 — the in-between write lost
    // because pages claimed seq=N+2 too). The seq is captured once
    // on the first page and carried through `snapshot_after`.
    let snapshot_seq: u64 = snapshot_after_parsed
        .as_ref()
        .and_then(|v| v.get("s").and_then(|s| s.as_u64()))
        .unwrap_or_else(|| ctx.change_log.current_seq());

    // Per-request scanned-row budget. The page row limit bounds rows
    // EMITTED, but a data-dependent sparse policy (e.g. `auth.userId ==
    // data.ownerId` on a large shared table) passes `check_entity_scan` and is
    // walked row-by-row, with most rows dropped at the per-row `check_entity_read`
    // fence — so `changes.len()` never reaches the emit limit and the loop would
    // scan the whole table in one request (DB-egress / request-timeout storm,
    // then a never-converging since=0 re-snapshot). Bound rows TOUCHED, not just
    // rows passed: break with a `snapshot_after` continuation once the budget is
    // hit so pagination is bounded by scan progress. Env-tunable for ops.
    let raw_scan_budget: usize = snapshot_scan_budget();

    // Read policies are evaluated PER ROW below, and a tenant gate asks the
    // same `exists(...)` question for every row of the scan — a real app
    // gating its stat tables on an active-subscription check was running three
    // identical subqueries per row, thousands per dashboard load, which
    // dominated the request. Memoize for the length of this scan only; the
    // guard drops with the request so nothing goes stale.
    let _exists_memo = pylon_policy::ExistsMemo::scope();

    let manifest = ctx.store.manifest();
    let auth_user = &manifest.auth.user;
    let mut changes: Vec<ChangeEvent> = Vec::new();
    let mut scanned: usize = 0;
    // (entity, ascending `after` id, rows emitted for the entity, backward
    // `before` id of a capped entity's tail walk)
    let mut next_after: Option<(String, Option<String>, usize, Option<String>)> = None;
    let resume_entity = snapshot_after.as_ref().map(|c| c.0.clone());
    let resume_after_id = snapshot_after.as_ref().and_then(|c| c.1.clone());
    // A capped entity's backward walk resumes before this id.
    let resume_before: Option<String> = snapshot_after_parsed
        .as_ref()
        .and_then(|v| v.get("b").and_then(|s| s.as_str()).map(str::to_string));
    // Rows already emitted for the entity we're resuming INTO. `sync_limit` is
    // a per-client-per-entity ceiling, and a snapshot spans however many
    // requests the page row limit forces — so a counter that reset each
    // request would let a big table emit `limit` rows per page and bound
    // nothing at all.
    let resume_emitted: usize = snapshot_after_parsed
        .as_ref()
        .and_then(|v| v.get("n").and_then(|n| n.as_u64()))
        .unwrap_or(0) as usize;
    let mut started = resume_entity.is_none();

    'outer: for entity in &manifest.entities {
        if entity.name.starts_with('_') {
            continue;
        }
        // `sync: false` entities (large server-queried catalogs) are never bulk-
        // replicated into the client — reached via search + by-id fetch instead.
        if !entity.sync {
            continue;
        }
        if !started {
            if Some(&entity.name) == resume_entity.as_ref() {
                started = true;
            } else {
                continue;
            }
        }
        // Entity-level read short-circuits — skip the ENTIRE entity without
        // reading a page, so a deny-all table never gets walked top-to-bottom
        // only to drop every row at the per-row fence below. For a large
        // append-only table (an audit log) that wasted read either dominates
        // DB egress or can't finish inside the request window, so the client
        // never sees `has_more: false` and re-snapshots `since=0` forever
        // (the 2026-06-03 pylon-cloud egress storm).
        //
        // (1) `is_read_statically_denied` — a literal `allowRead: "false"`
        // entity is never bulk-snapshotted to ANY caller, ADMIN INCLUDED.
        // The admin bypass that `check_entity_scan` honors does NOT apply to
        // bulk replication: an admin still reads the table via /api/entities
        // or a function, but the whole ever-growing table is not streamed
        // into their browser on every connect.
        if ctx.policy_engine.is_read_statically_denied(&entity.name) {
            continue;
        }
        // (2) Per-caller scan short-circuit (admin-bypassed): skip entities a
        // non-admin caller can't read at all (a failing data-independent auth
        // gate). Data-dependent tenant policies (`exists(...)`, `data.X ==
        // auth.Y`) return Allowed and are still scanned + per-row filtered.
        if !matches!(
            ctx.policy_engine
                .check_entity_scan(&entity.name, ctx.auth_ctx),
            pylon_policy::PolicyResult::Allowed
        ) {
            continue;
        }
        let resuming_this_entity = Some(&entity.name) == resume_entity.as_ref();
        let initial_after = if resuming_this_entity {
            resume_after_id.clone()
        } else {
            None
        };
        let mut entity_after = initial_after;
        let mut emitted_for_entity: usize = if resuming_this_entity {
            resume_emitted
        } else {
            0
        };

        // A capped entity takes the tail of the table, walking BACKWARDS from
        // the newest row, instead of walking it from the beginning: the cap
        // is aimed at time-series tables, and an id-ascending walk truncated
        // to `cap` would hand the replica the OLDEST rows.
        //
        // The walk is paged across requests like the ascending one: a
        // continuation carries the backward position (`b`) and the rows
        // emitted so far (`n`). It stops at `cap` visible rows or at the start
        // of the table. Pre-fix it stopped at the per-request scan budget and
        // emitted what it had with no continuation, so after a bulk import of
        // rows another caller sees, every in-scope row older than 50,000 of
        // them never reached the replica: a rep with ~7,400 in-scope
        // properties synced 70, with no error. Rows dropped by the policy
        // just mean fewer than `cap` arrive, which the contract allows:
        // `sync_limit` is a ceiling, not a quota.
        // Read only the rows the caller's scope can keep (the store runs
        // the scope in its query where it can); the checks below still
        // decide each row.
        let scope_filter = ctx
            .policy_engine
            .sync_scope_filter(&entity.name, ctx.auth_ctx);
        if let Some(cap) = entity.sync_limit {
            // Paged rather than one `list_last(cap)` fetch: the cap counts
            // VISIBLE rows, and visibility is decided per row by the read
            // policy. A single fetch would take the newest `cap` rows across
            // every tenant and then filter, so on a busy multi-tenant table a
            // quiet tenant could receive nothing at all.
            let mut before: Option<String> = if resuming_this_entity {
                resume_before.clone()
            } else {
                None
            };
            let mut supported = true;
            // This request's rows of the entity, newest first. Emitted
            // oldest-first below, like every other snapshot page.
            let mut batch: Vec<ChangeEvent> = Vec::new();
            // Where the walk paused, when it did: the continuation.
            let mut paused = false;
            'tail: while emitted_for_entity + batch.len() < cap {
                let read = match &scope_filter {
                    Some(filter) => ctx.store.list_last_filtered(
                        &entity.name,
                        before.as_deref(),
                        RAW_FETCH_CHUNK,
                        filter,
                    ),
                    None => ctx
                        .store
                        .list_last(&entity.name, before.as_deref(), RAW_FETCH_CHUNK),
                };
                let rows = match read {
                    Ok(Some(rows)) => rows,
                    // Store can't scan backwards — fall through to the
                    // ascending walk rather than failing the snapshot.
                    Ok(None) if batch.is_empty() && before.is_none() => {
                        supported = false;
                        break;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        tracing::error!(
                            entity = %entity.name,
                            error = ?e,
                            "sync_limit tail read failed; returning 503 for client retry"
                        );
                        let resume_payload = serde_json::json!({
                            "e": entity.name.clone(),
                            "a": serde_json::Value::Null,
                            "b": before.clone(),
                            "s": snapshot_seq,
                            "n": emitted_for_entity,
                        })
                        .to_string();
                        let body = serde_json::json!({
                                "error": {
                                    "code": "SNAPSHOT_PAGE_FAILED",
                                    "hint": "transient storage error mid-snapshot — retry with the same snapshot_after",
                                    "message": format!("snapshot tail read failed for entity `{}` before id={:?}", entity.name, before),
                                    "snapshot_after": url_encode(&resume_payload),
                                }
                            })
                            .to_string();
                        return (503, body);
                    }
                };
                if rows.is_empty() {
                    break;
                }
                let chunk_len = rows.len();
                for row in rows {
                    let Some(row_id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string)
                    else {
                        continue;
                    };
                    before = Some(row_id.clone());
                    scanned += 1;
                    let visible = matches!(
                        ctx.policy_engine
                            .check_sync_scope(&entity.name, ctx.auth_ctx, Some(&row)),
                        pylon_policy::PolicyResult::Allowed
                    ) && matches!(
                        ctx.policy_engine
                            .check_entity_read(&entity.name, ctx.auth_ctx, Some(&row)),
                        pylon_policy::PolicyResult::Allowed
                    );
                    if visible {
                        let event = ChangeEvent {
                            seq: snapshot_seq,
                            entity: entity.name.clone(),
                            row_id,
                            kind: ChangeKind::Insert,
                            data: Some(crate::project_row_for_replication(
                                manifest,
                                auth_user,
                                &entity.name,
                                row,
                            )),
                            prev_data: None,
                            timestamp: String::new(),
                        };
                        page.add(&event);
                        batch.push(event);
                        if emitted_for_entity + batch.len() >= cap {
                            break 'tail;
                        }
                        if page.full(changes.len() + batch.len()) {
                            paused = true;
                            break 'tail;
                        }
                    }
                    // Bound rows TOUCHED per request, as the ascending path
                    // does: pause with a continuation instead of stopping.
                    if scanned >= raw_scan_budget {
                        paused = true;
                        break 'tail;
                    }
                }
                if chunk_len < RAW_FETCH_CHUNK {
                    break;
                }
            }
            if supported {
                emitted_for_entity += batch.len();
                changes.extend(batch.into_iter().rev());
                if paused && emitted_for_entity < cap {
                    next_after = Some((entity.name.clone(), None, emitted_for_entity, before));
                    break 'outer;
                }
                continue;
            }
        }

        // Rows that come through a scope mostly pass: read them in larger
        // chunks.
        let chunk = match scope_filter {
            Some(_) => FILTERED_FETCH_CHUNK,
            None => RAW_FETCH_CHUNK,
        };
        loop {
            let read = match &scope_filter {
                Some(filter) => ctx.store.list_after_filtered(
                    &entity.name,
                    entity_after.as_deref(),
                    chunk,
                    filter,
                ),
                None => ctx
                    .store
                    .list_after(&entity.name, entity_after.as_deref(), chunk),
            };
            let raw = match read {
                Ok(r) => r,
                Err(e) => {
                    // Mid-snapshot read failure. The original code
                    // logged + `break`'d out of the entity loop —
                    // but if no later entity overflowed the batch
                    // limit, `has_more` ended up `false` and the
                    // cursor advanced to `snapshot_seq`. The
                    // truncated entity's missing rows became
                    // structurally invisible: pull retry wouldn't
                    // recover them because the cursor was past
                    // the snapshot.
                    //
                    // Return 503 with the SAME `snapshot_after`
                    // the client sent (or a freshly-encoded one
                    // pointing at where we got to in this entity)
                    // so retry resumes at the same row. Cursor
                    // MUST NOT advance for a truncated entity.
                    tracing::error!(
                        entity = %entity.name,
                        after = ?entity_after,
                        error = ?e,
                        "snapshot pagination list_after failed; returning 503 for client retry"
                    );
                    let resume_payload = serde_json::json!({
                        "e": entity.name.clone(),
                        "a": entity_after.clone(),
                        "s": snapshot_seq,
                    })
                    .to_string();
                    let body = serde_json::json!({
                            "error": {
                                "code": "SNAPSHOT_PAGE_FAILED",
                                "hint": "transient storage error mid-snapshot — retry with the same snapshot_after",
                                "message": format!("snapshot pagination failed for entity `{}` after id={:?}", entity.name, entity_after),
                                "snapshot_after": url_encode(&resume_payload),
                            }
                        })
                        .to_string();
                    return (503, body);
                }
            };
            if raw.is_empty() {
                break;
            }
            let raw_len = raw.len();
            for row in raw {
                let row_id = row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if row_id.is_empty() {
                    continue;
                }
                entity_after = Some(row_id.clone());
                scanned += 1;
                // Replication scope, layered ON TOP of the read policy and
                // checked first because it's the cheaper of the two on a
                // scoped entity (a bare field comparison vs. a policy that may
                // run an `exists(...)` subquery). It can only ever REMOVE rows
                // from a replica — the read policy below still decides what
                // the caller is allowed to see at all.
                if !matches!(
                    ctx.policy_engine
                        .check_sync_scope(&entity.name, ctx.auth_ctx, Some(&row)),
                    pylon_policy::PolicyResult::Allowed
                ) {
                    continue;
                }
                // Per-entity replication ceiling. Belt to the scope's braces:
                // a predicate that turns out not to bound growth still can't
                // flood the replica. Counted per entity, per request-chain,
                // and enforced BEFORE the row is emitted.
                if let Some(cap) = entity.sync_limit {
                    if emitted_for_entity >= cap {
                        // Stop this entity entirely — advancing the cursor
                        // past it rather than paginating a table we have
                        // already decided not to finish.
                        break;
                    }
                }
                if matches!(
                    ctx.policy_engine
                        .check_entity_read(&entity.name, ctx.auth_ctx, Some(&row)),
                    pylon_policy::PolicyResult::Allowed
                ) {
                    // Apply both projections in one pass: User entity
                    // allowlist + `serverOnly` field strip. Without the
                    // second, fields like `Org.stripeCustomerId.serverOnly()`
                    // leak through the snapshot path even though
                    // `/api/entities` enforces them.
                    let projected_data = Some(crate::project_row_for_replication(
                        manifest,
                        auth_user,
                        &entity.name,
                        row,
                    ));
                    changes.push(ChangeEvent {
                        seq: snapshot_seq,
                        entity: entity.name.clone(),
                        row_id,
                        kind: ChangeKind::Insert,
                        data: projected_data,
                        prev_data: None,
                        timestamp: String::new(),
                    });
                    page.add(changes.last().expect("just pushed"));
                    emitted_for_entity += 1;
                    if page.full(changes.len()) {
                        next_after = Some((
                            entity.name.clone(),
                            entity_after.clone(),
                            emitted_for_entity,
                            None,
                        ));
                        break 'outer;
                    }
                }
                // Bound rows TOUCHED per request — checked after emitting the
                // current row (so a passing row at the boundary is never
                // skipped). A sparse data-dependent policy that drops most rows
                // paginates by scan progress instead of walking the whole table.
                if scanned >= raw_scan_budget {
                    next_after = Some((
                        entity.name.clone(),
                        entity_after.clone(),
                        emitted_for_entity,
                        None,
                    ));
                    break 'outer;
                }
            }
            if raw_len < chunk {
                break;
            }
        }
    }

    // Stay at `last_seq: 0` while paginating so a follow-up pull with
    // since=0 still routes back to this branch. Only advance to the
    // captured snapshot_seq on the FINAL batch (no more snapshot
    // pages) so the client transitions to the change-log tail at the
    // right "as of" point.
    let has_more = next_after.is_some();
    let cursor_seq = if has_more { 0 } else { snapshot_seq };
    let next_after_str = next_after.map(|(e, a, n, b)| {
        // `n` carries per-entity emitted count so `sync_limit` survives
        // pagination — see resume_emitted. `b` is a capped entity's backward
        // position.
        let mut payload = serde_json::json!({"e": e, "a": a, "s": snapshot_seq, "n": n});
        if let Some(b) = b {
            payload["b"] = serde_json::json!(b);
        }
        let payload = payload.to_string();
        url_encode(&payload)
    });
    let mut resp = serde_json::json!({
        "changes": changes,
        "cursor": { "last_seq": cursor_seq },
        "has_more": has_more,
        "snapshot_after": next_after_str,
        // Every page of a snapshot says so: a catching-up client can be
        // answered with one (see `handle_delta_pull`) and must know.
        "snapshot": true,
    });
    if !has_more {
        // The entities this snapshot holds the complete visible set of. A
        // client that kept rows from before deletes the ones not sent (the
        // snapshot has no tombstones).
        let entities: Vec<&str> = manifest
            .entities
            .iter()
            .filter(|e| crate::is_replicated_entity(manifest, &e.name))
            .map(|e| e.name.as_str())
            .collect();
        resp["snapshot_entities"] = serde_json::json!(entities);
    }
    (200, resp.to_string())
}

/// Whether the client handles a snapshot in answer to a delta pull
/// (`snapshot_ok=1`). Older clients get the 410 instead, which makes them
/// wipe the replica and pull from 0.
fn accepts_snapshot_catch_up(url: &str) -> bool {
    url.split('?').nth(1).is_some_and(|qs| {
        qs.split('&')
            .any(|kv| kv == "snapshot_ok=1" || kv == "snapshot_ok=true")
    })
}

/// Test override for [`delta_scan_budget`] — same atomic-not-env
/// pattern (and rationale) as [`SNAPSHOT_SCAN_BUDGET`]. 0 = default.
static DELTA_SCAN_BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Override the per-request raw-event scan budget for delta pulls.
/// 0 restores the default.
pub fn set_delta_scan_budget(events: usize) {
    DELTA_SCAN_BUDGET.store(events, std::sync::atomic::Ordering::Relaxed);
}

/// Raw change-log events one delta request may SCAN while refilling a
/// page (see the refill loop below). Bounds request time when the
/// policy fence drops most events for this caller.
fn delta_scan_budget() -> usize {
    let overridden = DELTA_SCAN_BUDGET.load(std::sync::atomic::Ordering::Relaxed);
    if overridden > 0 {
        return overridden;
    }
    std::env::var("PYLON_DELTA_SCAN_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(10_000)
}

/// Test override for [`delta_resync_threshold`]. 0 = no override.
static DELTA_RESYNC_THRESHOLD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Override the far-behind cutover threshold. 0 restores the default.
pub fn set_delta_resync_threshold(gap: u64) {
    DELTA_RESYNC_THRESHOLD.store(gap, std::sync::atomic::Ordering::Relaxed);
}

/// Minimum cursor gap (in change-log events) before the far-behind
/// cutover even considers forcing a snapshot resync. 0 disables the
/// cutover entirely.
fn delta_resync_threshold() -> u64 {
    let overridden = DELTA_RESYNC_THRESHOLD.load(std::sync::atomic::Ordering::Relaxed);
    if overridden > 0 {
        return overridden;
    }
    match std::env::var("PYLON_DELTA_RESYNC_THRESHOLD") {
        Ok(v) => v.parse().unwrap_or(10_000),
        Err(_) => 10_000,
    }
}

/// Total rows across synced entities, cached process-wide for 60s.
/// Feeds the far-behind cutover: COUNT(*) per entity is not free on a
/// large table, and a fleet of stale clients reconnecting at once must
/// not turn the cheap "should we snapshot instead?" question into its
/// own COUNT storm. Best-effort: `None` when any count fails (backend
/// without aggregate support) — the caller then skips the cutover and
/// serves the delta as before.
fn total_synced_rows(ctx: &RouterContext) -> Option<u64> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    // Single-slot cache keyed by store identity: one process normally
    // serves one store, but test binaries (and any future multi-app
    // host) run several — a count cached for one store must never
    // answer for another.
    static CACHE: Mutex<Option<(usize, Instant, Option<u64>)>> = Mutex::new(None);
    let store_key = ctx.store as *const dyn crate::DataStore as *const () as usize;
    if let Ok(guard) = CACHE.lock() {
        if let Some((key, at, total)) = *guard {
            if key == store_key && at.elapsed() < Duration::from_secs(60) {
                return total;
            }
        }
    }
    let manifest = ctx.store.manifest();
    let mut total: u64 = 0;
    let mut ok = true;
    for entity in &manifest.entities {
        if !entity.sync || entity.name.starts_with('_') {
            continue;
        }
        match ctx
            .store
            .aggregate(&entity.name, &serde_json::json!({ "count": "*" }))
        {
            Ok(res) => {
                let count = res
                    .get("rows")
                    .and_then(|r| r.get(0))
                    .and_then(|row| row.get("count"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0);
                total = total.saturating_add(count);
            }
            Err(_) => {
                ok = false;
                break;
            }
        }
    }
    let result = if ok { Some(total) } else { None };
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((store_key, Instant::now(), result));
    }
    result
}

fn resync_required_body(since: u64, oldest_seq: u64) -> String {
    serde_json::json!({
        "error": {
            "code": "RESYNC_REQUIRED",
            "message": format!(
                "cursor last_seq={since} is older than the oldest retained seq={oldest_seq}; client must re-sync"
            ),
            "oldest_seq": oldest_seq,
        }
    })
    .to_string()
}

/// Catching-up client (since>0): replay the change-log tail since the
/// caller's cursor, with the read-policy fence applied to each event.
fn handle_delta_pull(ctx: &RouterContext, url: &str, since: u64) -> (u16, String) {
    // Far-behind cutover. Replaying a delta costs one event per CHANGE
    // since the cursor; a snapshot costs one event per CURRENT visible
    // row, with every intermediate update coalesced and deleted rows
    // absent. When the gap clearly exceeds what a full snapshot would
    // send, force the snapshot path via the same 410 the retention
    // window uses — the client already handles it (reset replica, keep
    // pending mutations, re-pull from 0).
    //
    // Deliberately conservative: total rows OVERSTATES the per-caller
    // snapshot (policies filter rows out), so the cutover only fires
    // when the delta is worse than even a whole-table snapshot. Gap ≥
    // threshold gates the row count, so routine catch-ups never pay
    // the COUNT.
    //
    // A client that accepts a snapshot catch-up (`snapshot_ok=1`) gets the
    // snapshot in this response instead of a 410: it keeps its replica on
    // screen, applies the current rows, deletes the ones the snapshot no
    // longer holds, and resumes the log from the snapshot's seq. That costs
    // nothing extra on the client, so the cutover fires as soon as the
    // delta is longer than a whole-table snapshot (not twice as long).
    let snapshot_ok = accepts_snapshot_catch_up(url);
    let threshold = delta_resync_threshold();
    if threshold > 0 {
        let current = ctx.change_log.current_seq();
        let gap = current.saturating_sub(since);
        if gap >= threshold {
            if let Some(total) = total_synced_rows(ctx) {
                if snapshot_ok && gap > total {
                    return handle_snapshot_pull(ctx, url);
                }
                if gap > total.saturating_mul(2) {
                    return (410, resync_required_body(since, current.saturating_add(1)));
                }
            }
        }
    }

    let manifest = ctx.store.manifest();
    // Delta rows must belong to the same public entity set as snapshots.
    // Data-dependent policies ask the same `exists(...)` question for
    // every event of the page; memoize for this request only, same as
    // the snapshot path.
    let _exists_memo = pylon_policy::ExistsMemo::scope();

    // Refill loop. `change_log.pull` pages RAW events, but the policy
    // fence below can drop most of them for this caller (other tenants'
    // writes, non-synced entities) — and the client pays a full round
    // trip per response regardless of how many events survive. So keep
    // scanning the log until a real page accumulates, the log is
    // exhausted, or the scan budget is hit. The response cursor always
    // advances to the last RAW event scanned, so filtered-out spans are
    // never re-scanned on the next request.
    let mut page_budget = PageBudget::new(requested_page_rows(url));
    // A larger page may scan proportionally more of the log.
    let scan_budget = delta_scan_budget().max(page_budget.rows * 10);
    let mut changes: Vec<ChangeEvent> = Vec::new();
    let mut cursor_seq = since;
    let mut has_more;
    let mut scanned: usize = 0;
    loop {
        let page = match ctx.change_log.pull(
            &SyncCursor {
                last_seq: cursor_seq,
            },
            page_budget.rows,
        ) {
            Ok(page) => page,
            Err(pylon_sync::PullError::ResyncRequired { oldest_seq, .. }) => {
                if changes.is_empty() && cursor_seq == since {
                    // The log no longer holds the cursor's events.
                    if snapshot_ok {
                        return handle_snapshot_pull(ctx, url);
                    }
                    return (410, resync_required_body(since, oldest_seq));
                }
                // Mid-refill resync signal with a valid partial page
                // already accumulated: return the page (contiguous from
                // `since`) with has_more so the NEXT request surfaces
                // the 410 cleanly on its own.
                has_more = true;
                break;
            }
        };
        scanned += page.changes.len();
        let advanced = page.cursor.last_seq > cursor_seq;
        cursor_seq = page.cursor.last_seq;
        // No forward progress (store-error no-progress page, or any
        // future path that returns an empty page with has_more): break
        // rather than spin the refill loop against the same cursor.
        has_more = page.has_more && advanced;
        for ev in page
            .changes
            .into_iter()
            .filter(|ev| crate::is_replicated_entity(manifest, &ev.entity))
            .filter_map(|ev| project_change_for_caller(ctx, ev))
        {
            page_budget.add(&ev);
            changes.push(ev);
        }
        if !has_more || page_budget.full(changes.len()) || scanned >= scan_budget {
            break;
        }
    }

    // Wire-level projection on every kept event (data +
    // prev_data) so server-only fields don't leak via the
    // visibility-flip path either.
    let auth_user = &manifest.auth.user;
    for ev in changes.iter_mut() {
        if let Some(data) = ev.data.take() {
            ev.data = Some(crate::project_row_for_replication(
                manifest, auth_user, &ev.entity, data,
            ));
        }
        if let Some(prev) = ev.prev_data.take() {
            ev.prev_data = Some(crate::project_row_for_replication(
                manifest, auth_user, &ev.entity, prev,
            ));
        }
    }
    let resp = pylon_sync::PullResponse {
        changes,
        cursor: SyncCursor {
            last_seq: cursor_seq,
        },
        has_more,
    };
    (
        200,
        serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
    )
}

/// Apply the read-policy fence to a single change event. Returns the
/// event the caller should see (post-projection — but projection
/// itself is applied by the caller) or `None` to drop it. Behavior:
///
///   - post allowed → keep as-is (with prev_data stripped)
///   - post denied AND Update AND prev allowed → synthesize a Delete
///     at the same seq so the caller drops the now-invisible row
///   - post denied otherwise → drop silently (callers can't tell a
///     row was filtered from a row that never existed)
fn project_change_for_caller(ctx: &RouterContext, ev: ChangeEvent) -> Option<ChangeEvent> {
    // "Visible" on the replication path means readable AND inside the
    // caller's sync scope. Folding the scope in here rather than filtering
    // afterwards is what makes a row LEAVING scope behave correctly: the
    // update/delete flip below already converts "was visible, now isn't"
    // into a Delete, so a client evicts the row instead of holding a copy
    // that will never be updated again.
    let visible = |row: Option<&serde_json::Value>| {
        matches!(
            ctx.policy_engine
                .check_entity_read(&ev.entity, ctx.auth_ctx, row),
            pylon_policy::PolicyResult::Allowed
        ) && matches!(
            ctx.policy_engine
                .check_sync_scope(&ev.entity, ctx.auth_ctx, row),
            pylon_policy::PolicyResult::Allowed
        )
    };
    let post_allowed = visible(ev.data.as_ref());
    if post_allowed {
        // Strip `prev_data` before shipping — it's a server-internal
        // field used only for the dual-check; leaving it on the wire
        // leaks the pre-update row to every recipient.
        return Some(ChangeEvent {
            prev_data: None,
            ..ev
        });
    }
    if matches!(ev.kind, ChangeKind::Update) && ev.prev_data.is_some() {
        let pre_allowed = visible(ev.prev_data.as_ref());
        if pre_allowed {
            return Some(ChangeEvent {
                seq: ev.seq,
                entity: ev.entity.clone(),
                row_id: ev.row_id.clone(),
                kind: ChangeKind::Delete,
                data: ev.prev_data.clone(),
                prev_data: None,
                timestamp: ev.timestamp.clone(),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// GDPR endpoints
// ---------------------------------------------------------------------------

fn handle_gdpr(ctx: &RouterContext, method: HttpMethod, url: &str) -> Option<(u16, String)> {
    let tail = url.strip_prefix("/api/admin/users/")?;
    let tail = tail.split('?').next().unwrap_or(tail);
    let (user_id, action) = tail.split_once('/')?;
    if user_id.is_empty() {
        return None;
    }
    if action == "export" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        if let Some(err) = crate::require_gdpr_tenant_scope(ctx, user_id) {
            return Some(err);
        }
        return Some(gdpr_export(ctx, user_id));
    }
    if action == "purge" && method == HttpMethod::Delete {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        if let Some(err) = crate::require_gdpr_tenant_scope(ctx, user_id) {
            return Some(err);
        }
        return Some(gdpr_purge(ctx, user_id));
    }
    None
}

// ---------------------------------------------------------------------------
// Push path
// ---------------------------------------------------------------------------

/// POST /api/sync/push — client-safe public write path. Each op runs
/// through the same mutation pipeline as the entity-API handlers
/// (apply_mutation), so a non-admin caller's write must pass the
/// action-specific policy check (insert/update/delete) AND any
/// registered before_* plugin hook (validation, tenant-scope, audit).
///
/// Invariant: partial-failure batches return the per-op result
/// envelope intact — successful ops apply and broadcast; failed ops
/// short-circuit with their error code without rolling back siblings.
fn handle_push(ctx: &RouterContext, body: &str) -> (u16, String) {
    let push_req: pylon_sync::PushRequest = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            );
        }
    };

    let mut applied = 0u32;
    let mut errors: Vec<String> = Vec::new();
    let mut deduped = 0u32;
    // Highest seq we assigned, so the response cursor reflects real
    // current_seq instead of `change_log.len()` which silently
    // diverges from truth as retention evicts old events.
    let mut max_seq: u64 = 0;
    // Per-op result envelope: { op_id, row_id, entity, kind, status,
    // seq?, error? }. Clients map each entry back to its op_id (or
    // array index when op_id is absent) to know exactly which
    // mutations applied, were deduplicated, or failed.
    let mut op_results: Vec<serde_json::Value> = Vec::with_capacity(push_req.changes.len());

    let manifest = ctx.store.manifest();

    for change in &push_req.changes {
        // SECURITY: gate framework-internal `_`-prefixed entities
        // (_Connection, _PylonJobs, _PylonWorkflows, _PylonSchemaVersion, …)
        // to admins. Apps register no policy for them, so the policy gate's
        // "no policy ⇒ Allowed for underscore entities" bypass (it trusts the
        // route edge to gate) would otherwise let ANY caller insert/update/
        // delete framework state through /api/sync/push. The entity REST
        // surface already gates this (entities.rs); the push surface must too.
        // 404 (not 403) so we don't confirm the table exists. Done BEFORE the
        // op-id claim so a rejected op leaves no in-flight claim behind.
        if change.entity.starts_with('_') && !ctx.auth_ctx.is_admin {
            errors.push(format!(
                "{} {}/{}: Entity not found",
                change_kind_label(&change.kind),
                change.entity,
                change.row_id
            ));
            op_results.push(serde_json::json!({
                "op_id": change.op_id,
                "entity": change.entity,
                "row_id": change.row_id,
                "kind": change_kind_label(&change.kind),
                "status": "error",
                "error": { "code": "NOT_FOUND", "message": "Entity not found" },
            }));
            continue;
        }
        // A `sync: false` entity is not in any client replica, so a
        // replica write to it is never echoed back and the client's next
        // reconcile drops its optimistic row. Refuse it with an error that
        // says what to do instead, rather than apply a write the client
        // will then lose from view.
        if !crate::is_replicated_entity(manifest, &change.entity) {
            let message = format!(
                "{} is sync: false, so it is not in the client replica and a replica \
                 write to it cannot be confirmed. Write it with a mutation or \
                 /api/entities/{}.",
                change.entity, change.entity
            );
            if ctx.is_dev {
                tracing::warn!("[sync] refused a client write: {message}");
            }
            errors.push(format!(
                "{} {}/{}: {message}",
                change_kind_label(&change.kind),
                change.entity,
                change.row_id
            ));
            op_results.push(serde_json::json!({
                "op_id": change.op_id,
                "entity": change.entity,
                "row_id": change.row_id,
                "kind": change_kind_label(&change.kind),
                "status": "error",
                "error": { "code": "ENTITY_NOT_SYNCED", "message": message },
            }));
            continue;
        }
        // SECURITY: readonly fields — including the owner stamped by
        // `field.owner()` and identity columns like orgId/tenantId/createdBy —
        // are immutable from the client. The PATCH entity route enforces this
        // via `reject_readonly_payload`; the sync-push surface is the path the
        // SDK actually steers local-first apps to, so it must enforce it too.
        // Without this gate a crafted Update could reassign ownership or flip a
        // tenant scope (IDOR). Server code (function handlers) writes readonly
        // fields through the mutation runtime, not this client route, so it is
        // unaffected. Admins bypass inside `reject_readonly_payload`. Done
        // BEFORE the op-id claim so a rejected op leaves no in-flight claim.
        if matches!(change.kind, ChangeKind::Update) {
            if let Some(data) = change.data.as_ref() {
                if let Err((code, message)) =
                    crate::reject_readonly_payload(manifest, &change.entity, data, ctx.auth_ctx)
                {
                    errors.push(format!(
                        "update {}/{}: {message}",
                        change.entity, change.row_id
                    ));
                    op_results.push(serde_json::json!({
                        "op_id": change.op_id,
                        "entity": change.entity,
                        "row_id": change.row_id,
                        "kind": change_kind_label(&change.kind),
                        "status": "error",
                        "error": { "code": code, "message": message },
                    }));
                    continue;
                }
            }
        }
        // Tristate op-id state machine:
        //   Proceed       → run the write; on failure forget
        //   InFlight      → concurrent writer mid-flight; respond
        //                   "pending" so the client retries when
        //                   the first writer commits
        //   Replayed{seq} → first write already applied; return
        //                   cached seq so the client's optimistic
        //                   row adopts the canonical seq instead of
        //                   waiting for the WS rebroadcast
        if let Some(ref op_id) = change.op_id {
            match ctx.change_log.claim_op_id(op_id) {
                pylon_sync::OpClaim::Proceed => {}
                pylon_sync::OpClaim::InFlight => {
                    deduped += 1;
                    op_results.push(serde_json::json!({
                        "op_id": op_id,
                        "entity": change.entity,
                        "row_id": change.row_id,
                        "kind": change_kind_label(&change.kind),
                        "status": "pending",
                    }));
                    continue;
                }
                pylon_sync::OpClaim::Replayed { seq } => {
                    deduped += 1;
                    if seq > max_seq {
                        max_seq = seq;
                    }
                    op_results.push(serde_json::json!({
                        "op_id": op_id,
                        "entity": change.entity,
                        "row_id": change.row_id,
                        "kind": change_kind_label(&change.kind),
                        "status": "replayed",
                        "seq": seq,
                    }));
                    continue;
                }
            }
        }
        // Map the wire change kind onto a typed MutationOp so the
        // shared pipeline runs identical policy + plugin + reread +
        // broadcast logic as the entity REST handlers and
        // /api/link/unlink. Returns the assigned seq (Some on apply,
        // None when the reread missed and no event was appended).
        let mctx = MutationCtx::from_router(ctx);
        let op_id_opt = change.op_id.clone();
        let kind_label = change_kind_label(&change.kind);
        // Wrap the mutation pipeline so a panic inside it (a plugin/policy/
        // store `.unwrap()`) can't unwind past the complete/forget bookkeeping
        // below and leave this op_id stuck `Pending` forever — every client
        // retry would then wedge on `InFlight` until 10k other ops evict it.
        // A caught panic becomes a retryable 500 and the `Err` branch frees the
        // op_id. `AssertUnwindSafe` is sound here: on panic we read no mutation
        // state (the store mutation runs in a transaction that rolls back) — we
        // only free the op_id and return a fixed error.
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_push_op(&mctx, change)
        })) {
            Ok(o) => o,
            Err(_panic) => Err(MutationError::Hook {
                status: 500,
                code: "INTERNAL_ERROR".into(),
                message: "internal error processing change".into(),
            }),
        };

        let result_envelope = |status: &str,
                               row_id: Option<&str>,
                               seq: Option<u64>,
                               error: Option<(String, String)>|
         -> serde_json::Value {
            let mut e = serde_json::json!({
                "op_id": op_id_opt,
                "entity": change.entity,
                "row_id": row_id.unwrap_or(&change.row_id),
                "kind": kind_label,
                "status": status,
            });
            if let Some(s) = seq {
                e["seq"] = serde_json::Value::from(s);
            }
            if let Some((code, message)) = error {
                e["error"] = serde_json::json!({
                    "code": code,
                    "message": message,
                });
            }
            e
        };

        match outcome {
            Ok(out) => {
                if out.seq > max_seq {
                    max_seq = out.seq;
                }
                applied += 1;
                op_results.push(result_envelope(
                    "applied",
                    Some(&out.row_id),
                    Some(out.seq),
                    None,
                ));
                if let Some(ref op_id) = op_id_opt {
                    ctx.change_log.complete_op_id(op_id, out.seq);
                }
            }
            Err(err) => {
                let (_status, code, message) = crate::mutate::error_response(&err);
                let display = format!(
                    "{} {}/{}: {}",
                    kind_label, change.entity, change.row_id, message
                );
                errors.push(display);
                op_results.push(result_envelope("error", None, None, Some((code, message))));
                if let Some(ref op_id) = op_id_opt {
                    ctx.change_log.forget_op_id(op_id);
                }
            }
        }
    }

    // `current_seq()` is the authoritative position even when retention
    // has evicted older entries; `len()` would drift below the real
    // cursor as events scroll off the tail.
    let cursor_seq = if max_seq > 0 {
        max_seq
    } else {
        ctx.change_log.current_seq()
    };
    (
        200,
        serde_json::json!({
            "applied": applied,
            "deduped": deduped,
            "errors": errors,
            // Per-op result envelope. Clients should prefer this over
            // the count fields for status mapping. Count fields are
            // kept for backwards compat with older SDKs.
            "results": op_results,
            "cursor": {"last_seq": cursor_seq}
        })
        .to_string(),
    )
}

fn change_kind_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Insert => "insert",
        ChangeKind::Update => "update",
        ChangeKind::Delete => "delete",
    }
}

fn run_push_op(
    mctx: &MutationCtx,
    change: &pylon_sync::ClientChange,
) -> Result<crate::mutate::MutationOutcome, MutationError> {
    match change.kind {
        ChangeKind::Insert => match change.data.as_ref() {
            Some(data) => apply_mutation(
                mctx,
                MutationOp::Insert {
                    entity: &change.entity,
                    data,
                },
            ),
            None => Err(MutationError::Store {
                code: "INVALID_INPUT".into(),
                message: "insert requires data".into(),
            }),
        },
        ChangeKind::Update => match change.data.as_ref() {
            Some(data) => apply_mutation(
                mctx,
                MutationOp::Update {
                    entity: &change.entity,
                    row_id: &change.row_id,
                    data,
                },
            ),
            None => Err(MutationError::Store {
                code: "INVALID_INPUT".into(),
                message: "update requires data".into(),
            }),
        },
        ChangeKind::Delete => apply_mutation(
            mctx,
            MutationOp::Delete {
                entity: &change.entity,
                row_id: &change.row_id,
            },
        ),
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn requested_page_rows_is_clamped() {
        assert_eq!(
            requested_page_rows("/api/sync/pull?since=4"),
            DEFAULT_PAGE_ROWS
        );
        assert_eq!(
            requested_page_rows("/api/sync/pull?since=4&limit=4000"),
            4000
        );
        assert_eq!(
            requested_page_rows("/api/sync/pull?limit=999999&since=4"),
            MAX_PAGE_ROWS
        );
        assert_eq!(requested_page_rows("/api/sync/pull?since=4&limit=0"), 1);
        assert_eq!(
            requested_page_rows("/api/sync/pull?since=4&limit=x"),
            DEFAULT_PAGE_ROWS
        );
    }

    fn event(data: serde_json::Value) -> ChangeEvent {
        ChangeEvent {
            seq: 1,
            entity: "Property".into(),
            row_id: "p1".into(),
            kind: ChangeKind::Insert,
            data: Some(data),
            prev_data: None,
            timestamp: String::new(),
        }
    }

    #[test]
    fn a_page_ends_at_its_rows_or_its_bytes() {
        let mut by_rows = PageBudget::with_bytes(3, usize::MAX);
        for n in 1..=3 {
            by_rows.add(&event(serde_json::json!({ "id": "p1" })));
            assert_eq!(by_rows.full(n), n == 3);
        }

        // Wide rows: 1 KB of data each, against a 4 KB budget.
        let wide = serde_json::json!({ "notes": "x".repeat(1000) });
        let mut by_bytes = PageBudget::with_bytes(10_000, 4096);
        let mut rows = 0;
        while !by_bytes.full(rows) {
            by_bytes.add(&event(wide.clone()));
            rows += 1;
        }
        assert_eq!(rows, 4);
    }
}
