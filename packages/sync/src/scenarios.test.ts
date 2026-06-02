// Scenario tests for the sync engine. Each test reads like a story
// of timed events — set up the server, boot the engine, drive
// state changes through the harness, and assert at each step.
//
// Every test here pins a real fix the engine has shipped. When one
// fails, look at the commit named in the comment for context.
//
// Hardening principle: every assertion has to FAIL if the named fix
// is reverted. Tests that pass under both the fixed and broken engine
// are decorative and not allowed here.

import { afterEach, describe, expect, test } from "bun:test";

import { createTestEnv, type TestEnv } from "./test-harness";

const tenantScopedVisibility = (
  _e: string,
  rows: Array<Record<string, unknown>>,
  auth: { tenantId: string | null },
) => rows.filter((r) => r.orgId === auth.tenantId);

describe("sync scenarios", () => {
  let env: TestEnv | null = null;

  afterEach(async () => {
    if (env) {
      await env.dispose();
      env = null;
    }
  });

  // Pins: 2a79897e (drop first-load reconcile that races against
  // selectOrg). Before this fix, the engine's start() ran reconcile
  // immediately after the first pull. If the page's bootstrap effect
  // called selectOrg AFTER mount, reconcile would already have
  // fetched every entity under tenant=null, gotten zero rows, and
  // tombstoned the IndexedDB-hydrated cache before selectOrg
  // landed. Symptom: rows render briefly then "flash away."
  //
  // Runs under `transport: "poll"` so the WS-onopen reconcile
  // (which is a separate path with its own race semantics) doesn't
  // muddle the isolation. The fix being pinned is specifically the
  // reconcile call that USED to live in start() and now doesn't.
  // If that reconcile is reintroduced, this test fails — it counts
  // /api/entities/Recording/cursor fetches and expects ZERO during
  // start(), since the cached cursor list ("Recording") would be
  // swept under tenant=null.
  test("first-load reconcile does not race selectOrg", async () => {
    env = createTestEnv({
      visible: tenantScopedVisibility,
      transport: "poll",
    });
    env.server.seed("Recording", [
      { id: "r1", orgId: "org-a", title: "alive" },
    ]);
    env.signIn({ userId: "u1", tenantId: null });

    env.engine.store.applyChange({
      seq: 0,
      entity: "Recording",
      row_id: "r1",
      kind: "insert",
      data: { id: "r1", orgId: "org-a", title: "alive" },
      timestamp: "",
    });

    const beforeStart = env.transport.fetchCount();
    await env.start();
    // Snapshot fetch count immediately after start to assert the
    // in-start path didn't fetch the entity cursor endpoint.
    const afterStart = env.transport.fetchCount();
    expect(env.engine.store.list("Recording")).toHaveLength(1);

    // The in-start sequence should be /api/auth/me + /api/sync/pull
    // ONLY. Reverting 2a79897e adds /api/entities/Recording/cursor.
    expect(afterStart - beforeStart).toBeLessThanOrEqual(2);

    env.selectOrg("org-a");
    await env.engine.notifySessionChanged();
    await env.flush();

    expect(env.engine.store.list("Recording")).toHaveLength(1);
  });

  // Pins: 5b104b34 (null→tenant first-resolution doesn't reset replica).
  // refreshResolvedSession was calling resetReplica on every tenant
  // change. When the engine started under tenant=null and selectOrg
  // later set tenant=X, the "change" wiped local rows even though
  // they were the right rows all along.
  //
  // Poll transport keeps the WS-onopen reconcile from muddling the
  // assertion — the fix being pinned lives in refreshResolvedSession,
  // not in any WS path. `notifySessionChanged()` drives the engine
  // refresh deterministically.
  test("null → tenant flip does not reset cached rows", async () => {
    env = createTestEnv({
      visible: tenantScopedVisibility,
      transport: "poll",
    });
    env.server.seed("Recording", [{ id: "r1", orgId: "org-a" }]);
    env.signIn({ userId: "u1", tenantId: null });

    env.engine.store.applyChange({
      seq: 0,
      entity: "Recording",
      row_id: "r1",
      kind: "insert",
      data: { id: "r1", orgId: "org-a" },
      timestamp: "",
    });

    await env.start();
    expect(env.engine.store.list("Recording")).toHaveLength(1);

    env.selectOrg("org-a"); // null → X
    await env.engine.notifySessionChanged();
    await env.flush();

    expect(env.engine.store.list("Recording")).toHaveLength(1);
  });

  // Pins: dc43edc6 (reconcile bails on mid-fetch session flip).
  // The cursor guard already covered "WS event landed mid-fetch";
  // this fix added the parallel guard for session changes — if the
  // resolved session changes between fetch dispatch and fetch
  // return, the apply pass is skipped (otherwise rows filtered
  // under the OLD session would tombstone rows that ARE visible
  // under the NEW session).
  //
  // We trigger the race via the `beforeListEntityRows` hook: when
  // the engine fetches /api/entities/Recording/cursor under tenant=
  // org-a, the hook flips the session to org-b right before the
  // server serializes the response. The engine's session-signature
  // check after the await catches the flip and skips the apply.
  test("reconcile bails when session flips mid-fetch", async () => {
    let firstFetch = true;
    env = createTestEnv({
      // Visibility is gated by an `editor` role. Test starts with
      // role=editor (row visible), then the hook strips the role
      // mid-fetch — server now returns []. If the engine's
      // session-guard works, it sees the signature flip and skips
      // the apply; r1 stays in the store. Without the guard, the
      // empty server response would tombstone r1.
      visible: (_e, rows, auth) =>
        auth.roles.includes("editor") ? rows : [],
      transport: "poll",
      beforeListEntityRows: async (entity) => {
        if (entity === "Recording" && firstFetch) {
          firstFetch = false;
          if (env?.token) env.server.mutateSession(env.token, { roles: [] });
          await env!.engine.notifySessionChanged();
        }
      },
    });
    env.server.seed("Recording", [
      { id: "r1", orgId: "org-a", title: "alive" },
    ]);
    env.signIn({ userId: "u1", tenantId: "org-a", roles: ["editor"] });
    await env.start();
    await env.flush();
    expect(env.engine.store.list("Recording")).toHaveLength(1);

    await env.engine.reconcile(["Recording"]);
    await env.flush();

    // Role change doesn't trigger resetReplica (only tenant flips
    // do). So the only way r1 survives is via the session-guard
    // skipping the apply of the empty server snapshot.
    expect(env.engine.store.get("Recording", "r1")).not.toBeNull();
  });

  // Pins: 38225996 (useQuery skips loading flash on cached refresh).
  // isHydrated() flips true after IndexedDB hydration settles —
  // even when the disk replica was empty. Without that, a freshly-
  // empty entity stuck loading=true forever.
  test("isHydrated() flips true after start() even with empty replica", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    expect(env.engine.isHydrated()).toBe(false);
    await env.start();
    expect(env.engine.isHydrated()).toBe(true);
  });

  // Live server pushes propagate via WS without polling.
  test("server-side insert reaches the engine via WS", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    expect(env.engine.store.list("Note")).toHaveLength(0);

    env.server.insert("Note", { id: "n1", title: "hello" });
    await env.flush(50);

    expect(env.engine.store.list("Note")).toHaveLength(1);
    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "hello" });
  });

  // Live server-side delete propagates and removes the local row.
  test("server-side delete tombstones the local row", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.insert("Note", { id: "n1", title: "hello" });
    await env.flush(50);
    expect(env.engine.store.list("Note")).toHaveLength(1);

    env.server.delete("Note", "n1");
    await env.flush(50);
    expect(env.engine.store.list("Note")).toHaveLength(0);
  });

  // Server-side update lands and overwrites the local row (not just
  // append + duplicate). Pins the "ws update kind" path of
  // enqueueApply that's distinct from insert.
  test("server-side update propagates and overwrites local row", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.insert("Note", { id: "n1", title: "hello" });
    await env.flush(50);
    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "hello" });

    env.server.update("Note", "n1", { title: "updated" });
    await env.flush(50);
    expect(env.engine.store.list("Note")).toHaveLength(1);
    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "updated" });
  });

  // Multi-tenant policy isolation: rows scoped to org-a are invisible
  // to a session pinned to org-b on pull. Server-filtered, not just
  // client-filtered. Seeded rows now ARE in the log, so pull at
  // since=0 returns the org-b row without an explicit reconcile —
  // exactly the way production clients discover initial state.
  test("multi-tenant: org-b session cannot see org-a's rows on pull", async () => {
    env = createTestEnv({ visible: tenantScopedVisibility });
    env.server.seed("Recording", [
      { id: "a1", orgId: "org-a" },
      { id: "b1", orgId: "org-b" },
    ]);

    env.signIn({ userId: "u-b", tenantId: "org-b" });
    await env.start();

    const rows = env.engine.store.list("Recording");
    expect(rows).toHaveLength(1);
    expect((rows[0] as { id?: string }).id).toBe("b1");
  });

  // Real org switch X → Y must reset the replica AND re-pull under
  // the new tenant. Pins the second half of 5b104b34 — the
  // X-to-X-prime path that does call resetReplica + pull(). No
  // manual `engine.reconcile(...)` after the switch: if the engine
  // doesn't drive the re-pull itself, this test fails.
  test("real org switch (X → Y) resets replica and re-pulls", async () => {
    env = createTestEnv({ visible: tenantScopedVisibility });
    env.server.seed("Recording", [
      { id: "a1", orgId: "org-a", title: "alice" },
      { id: "b1", orgId: "org-b", title: "bob" },
    ]);
    env.signIn({ userId: "u1", tenantId: "org-a" });
    await env.start();
    await env.flush();
    expect(env.engine.store.get("Recording", "a1")).not.toBeNull();
    expect(env.engine.store.get("Recording", "b1")).toBeNull();

    env.selectOrg("org-b");
    // Refresh runs async via the WS session-changed envelope; give
    // it room to call /api/auth/me, resetReplica, and re-pull.
    await env.flush(100);

    expect(env.engine.store.get("Recording", "a1")).toBeNull();
    expect(env.engine.store.get("Recording", "b1")).not.toBeNull();
  });

  // Reactive query subscription delivers a result over WS AND emits
  // the outbound `reactive-subscribe` envelope. Both directions of
  // the wire contract are pinned: the engine must SEND the
  // subscribe message (without it the server's ReactiveRegistry
  // never sees the spec, and the engine receives no results), and
  // it must ROUTE the resulting `reactive-result` envelope back to
  // the local handler.
  test("reactive-subscribe is sent over WS and reactive-result is routed", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    let delivered: unknown = null;
    env.engine.subscribeReactive("sub-1", "myQuery", { foo: 1 }, (msg) => {
      if (msg.kind === "result") delivered = msg.result;
    });
    // The engine sends `reactive-subscribe` over the WS. Give the
    // mock WS open promise a chance to flush.
    await env.flush();
    const sent = env.server.receivedWsMessages.find(
      (entry) =>
        entry.userId === "u1" &&
        typeof entry.msg === "object" &&
        entry.msg !== null &&
        (entry.msg as { type?: string }).type === "reactive-subscribe" &&
        (entry.msg as { sub_id?: string }).sub_id === "sub-1",
    );
    expect(sent).toBeDefined();

    env.server.pushToUser("u1", {
      type: "reactive-result",
      sub_id: "sub-1",
      result: { rows: [{ id: "x" }] },
    });
    await env.flush();

    expect(delivered).toMatchObject({ rows: [{ id: "x" }] });
  });

  // Tombstone seq-guard: once a row is deleted at seq=N, a stale
  // insert/update with seq < N must NOT resurrect it. Pins the
  // tombstone bookkeeping in LocalStore.applyChange().
  test("tombstoned row is not resurrected by stale lower-seq event", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    env.engine.store.applyChange({
      seq: 100,
      entity: "Note",
      row_id: "n1",
      kind: "delete",
      data: { id: "n1" },
      timestamp: "",
    });
    expect(env.engine.store.get("Note", "n1")).toBeNull();

    env.engine.store.applyChange({
      seq: 50,
      entity: "Note",
      row_id: "n1",
      kind: "insert",
      data: { id: "n1", title: "STALE" },
      timestamp: "",
    });
    expect(env.engine.store.get("Note", "n1")).toBeNull();

    env.engine.store.applyChange({
      seq: 200,
      entity: "Note",
      row_id: "n1",
      kind: "insert",
      data: { id: "n1", title: "fresh" },
      timestamp: "",
    });
    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "fresh" });
  });

  // Stale WS retransmit at the engine level: an event with seq <=
  // cursor.last_seq must be dropped by enqueueApply before the
  // store ever sees it. Pins the per-event monotonic filter at
  // index.ts:580. Sending the WS event with seq=1 after the cursor
  // has advanced past it should be a no-op.
  test("WS retransmit with stale seq is dropped by enqueueApply", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    env.server.insert("Note", { id: "n1", title: "fresh" });
    env.server.insert("Note", { id: "n2", title: "two" });
    await env.flush(50);
    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "fresh" });

    // Push a manual envelope claiming to be seq=1 with stale data
    // for n1 — engine's monotonic filter sees seq=1 <= cursor and
    // drops it before the store applies it.
    env.server.pushToUser("u1", {
      seq: 1,
      entity: "Note",
      row_id: "n1",
      kind: "update",
      data: { id: "n1", title: "STALE" },
      timestamp: "",
    });
    await env.flush();

    expect(env.engine.store.get("Note", "n1")).toMatchObject({ title: "fresh" });
  });

  // serverOnly field stripping (pins commits cf71992f / 109bed48):
  // fields the schema declares server-only must never reach the
  // client replica. The harness's `projectRow` strips `secret`
  // before serialization, mimicking how production projects rows
  // on the wire path. Engine should never observe the stripped
  // field, neither via pull nor via WS.
  test("serverOnly field is stripped before reaching the engine", async () => {
    env = createTestEnv({
      projectRow: (_entity, row) => {
        const r = row as Record<string, unknown>;
        const { secret: _omit, ...rest } = r;
        return rest as typeof row;
      },
    });
    env.server.seed("Note", [
      { id: "n1", title: "hi", secret: "shh" },
    ]);
    env.signIn({ userId: "u1" });
    await env.start();
    await env.flush();

    const row = env.engine.store.get("Note", "n1") as Record<string, unknown> | null;
    expect(row).not.toBeNull();
    expect(row).toMatchObject({ title: "hi" });
    expect(row).not.toHaveProperty("secret");

    // Also verify WS-delivered updates are stripped.
    env.server.update("Note", "n1", { title: "edited", secret: "leak" });
    await env.flush(50);
    const after = env.engine.store.get("Note", "n1") as Record<string, unknown> | null;
    expect(after).toMatchObject({ title: "edited" });
    expect(after).not.toHaveProperty("secret");
  });

  // 410 RESYNC_REQUIRED handler (pins the cursor-mismatch recovery
  // path): on 410, engine resets the replica and re-pulls from
  // seq=0. Without the handler the cursor stays ahead of the
  // server forever and pulls return empty.
  test("410 RESYNC_REQUIRED triggers resetReplica + re-pull", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "before" }]);
    await env.start();
    await env.flush();
    expect(env.engine.store.list("Note")).toHaveLength(1);

    // Push a stale local row that should be cleared by resetReplica.
    env.engine.store.applyChange({
      seq: 99999,
      entity: "Note",
      row_id: "stale-only",
      kind: "insert",
      data: { id: "stale-only", title: "ghost" },
      timestamp: "",
    });
    expect(env.engine.store.get("Note", "stale-only")).not.toBeNull();

    // Add a fresh canonical row so the post-410 re-pull has new
    // state to deliver, then prime the 410 and trigger a pull.
    env.server.insert("Note", { id: "n2", title: "after-410" });
    env.server.primeNextPullStatus(410);
    await env.engine.pull();
    await env.flush();

    // After 410, ghost is cleared (resetReplica wiped local-only
    // rows) and canonical rows are present (re-pull from seq=0).
    expect(env.engine.store.get("Note", "stale-only")).toBeNull();
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();
    expect(env.engine.store.get("Note", "n2")).not.toBeNull();
  });

  // EGRESS STORM GUARD (pins the circuit-breaker fix). When every delta
  // pull 410s but snapshots succeed — a client bouncing between cluster
  // instances whose in-memory change logs diverge — the client must
  // snapshot ONCE then back off, NOT re-snapshot on every pull.
  //
  // The bug: the breaker reset `consecutive_410s = 0` on every
  // successful pull, including the cursor=0 snapshot a 410 triggers. So
  // a 410 → full snapshot → 410 ping-pong reset the breaker each cycle
  // and the backoff never engaged. The result was a full `select *`
  // snapshot of EVERY entity ~every 3 seconds, indefinitely — a ~280GB
  // PlanetScale egress bill. The fix clears the breaker only on a
  // successful DELTA pull (cursor stable), never on the snapshot itself.
  test("repeated delta 410s back off instead of re-snapshotting (egress storm)", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "x" }]);
    await env.start();
    await env.flush();

    // Baseline: start() did exactly one snapshot pull.
    const before = env.server.snapshotPullCount;

    // Now every delta pull 410s (snapshots still succeed).
    env.server.force410OnDelta = true;
    for (let i = 0; i < 6; i++) {
      await env.engine.pull();
      await env.flush();
    }

    // Exactly ONE additional snapshot (the first resync). Every pull
    // after that 410s on the delta and routes to exponential backoff —
    // no further full snapshots. Pre-fix this was 6 (one per pull).
    expect(env.server.snapshotPullCount - before).toBe(1);
  });

  // EMPTY-ENTITY RECONCILE GAP (pins observeEntity). A server row in an
  // entity the local replica has NEVER cached stays invisible: useQuery
  // reads the empty local store, a delta pull can't recover a row
  // created before the cursor, and the no-arg reconcile sweeps only
  // entities with ≥1 local row (store.entityNames()) — so it skips the
  // empty entity entirely. The user hit this as "I attached the domain
  // but the Domains list is empty"; clearing IndexedDB (cursor→0→
  // snapshot) was the only recovery. observeEntity (called by useQuery
  // on mount) adds the entity to the sweep + fires a one-shot fetch.
  test("observing an entity recovers a server row the local cache never had", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    // Client has rows for Note, but none for Domain.
    env.server.seed("Note", [{ id: "n1", title: "x" }]);
    await env.start();
    await env.flush();
    expect(env.engine.store.list("Domain")).toHaveLength(0);

    // A Domain row exists server-side but was never delivered to this
    // client (created on another surface / a missed insert). Inserted
    // after start so the initial snapshot didn't include it; poll
    // transport means no auto-delivery.
    env.server.insert("Domain", { id: "d1", host: "chat.example.com" });

    // A no-arg reconcile sweeps only entities with local rows (Note),
    // so it never touches Domain — the row stays invisible. The bug.
    await env.engine.reconcile(["Note"]);
    await env.flush();
    expect(env.engine.store.get("Domain", "d1")).toBeNull();

    // observeEntity (what useQuery now calls on mount) adds Domain to
    // the sweep and fires a one-shot scoped reconcile — the row appears
    // without a cache clear.
    env.engine.observeEntity("Domain");
    await env.flush();
    expect(env.engine.store.get("Domain", "d1")).not.toBeNull();
  });

  // TAIL-PULL DEADLOCK (pins index.ts:1310). When a delta pull reports
  // has_more (the catch-up exceeds the server's per-pull limit), pullInner
  // drains the tail by recursing. Pre-fix it called the PUBLIC pull(),
  // which re-enters the op queue under key "pull" and coalesces onto the
  // promise it's currently running inside → permanent self-deadlock that
  // bricks the whole pull path. Post-fix it recurses into pullInner().
  test("delta pull with has_more drains the tail without self-deadlocking", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "a" }]);
    await env.start();
    await env.flush();

    // A later change exists; the next delta pull reports has_more once,
    // forcing the tail recursion.
    env.server.insert("Note", { id: "n2", title: "b" });
    env.server.primeNextPullHasMore();

    // Must complete, not hang. Pre-fix the pull never resolves and the
    // race rejects with "pull deadlocked".
    const e = env;
    await Promise.race([
      e.engine.pull().then(() => e.flush()),
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error("pull deadlocked")), 3000),
      ),
    ]);
    expect(env.engine.store.get("Note", "n2")).not.toBeNull();
  });

  // OFFLINE WRITES (pins the transient/permanent split in pushInner).
  // A push that fails with a NETWORK error (offline — fetch rejects, no
  // HTTP status) must keep the mutation `pending` and the optimistic
  // ghost intact, then re-send on reconnect. Pre-fix every transport
  // failure marked the mutation `failed` + rolled back the ghost, so an
  // offline insert vanished and was never re-sent.
  test("offline push keeps the mutation pending and the ghost intact (not failed)", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    await env.flush();

    env.server.primeNextPushOutcome({ kind: "network" });
    const id = await env.engine.insert("Note", { title: "offline" });
    await env.flush();

    // Offline (network throw, no status): the ghost survives and the
    // mutation stays PENDING — not `failed`. Pre-fix it was marked failed
    // and the ghost rolled back, so the offline write vanished and (being
    // `failed`, not `pending`) was never re-sent. Pending is exactly what
    // makes the standard reconnect push re-ship it. (insert() returns the
    // ROW id; match the mutation by row_id, not its op_id.)
    expect(env.engine.store.get("Note", id)).not.toBeNull();
    const mine = (env.engine.mutations as unknown as {
      queue: { change: { row_id: string }; status: string }[];
    }).queue.filter((m) => m.change.row_id === id);
    expect(mine).toHaveLength(1);
    expect(mine[0]!.status).toBe("pending");
  });

  // FAILED UPDATE ROLLBACK (pins failPushedMutation update path). A
  // PERMANENT rejection (403) of an update must restore the row to its
  // pre-update value. Pre-fix only inserts rolled back; a rejected update
  // left the local row permanently wrong.
  test("a permanently-rejected update restores the prior value", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "original" }]);
    await env.start();
    await env.flush();

    env.server.primeNextPushOutcome({ kind: "status", status: 403 });
    await env.engine.update("Note", "n1", { title: "edited" });
    await env.flush();

    const row = env.engine.store.get("Note", "n1") as { title?: string } | null;
    expect(row?.title).toBe("original");
  });

  // FAILED DELETE ROLLBACK (pins failPushedMutation delete path). A
  // rejected delete must bring the row back AND clear the optimistic
  // tombstone, so a later server insert of the same id isn't fenced out.
  // Pre-fix the row vanished and the tombstone blocked re-creation
  // forever.
  test("a permanently-rejected delete restores the row and un-fences it", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "keep" }]);
    await env.start();
    await env.flush();

    env.server.primeNextPushOutcome({ kind: "status", status: 403 });
    await env.engine.delete("Note", "n1");
    await env.flush();
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();

    // Un-fenced: a server insert of the same id now lands.
    env.server.insert("Note", { id: "n1", title: "server-updated" });
    await env.engine.pull();
    await env.flush();
    const row = env.engine.store.get("Note", "n1") as { title?: string } | null;
    expect(row?.title).toBe("server-updated");
  });

  // CROSS-IDENTITY MUTATION WIPE (pins resetReplica wipeMutations). An
  // identity flip (token / tenant change) must DROP the outgoing
  // identity's pending offline writes — replaying them under the new
  // session would attribute one user's writes to another. A same-user 410
  // RESYNC must KEEP them (they survive the snapshot refresh).
  test("an identity-flip reset wipes the outgoing identity's pending writes", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    await env.flush();

    // Queue an offline write — network failure keeps it pending.
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Note", { title: "draft" });
    await env.flush();
    const queue = (env.engine.mutations as unknown as { queue: unknown[] }).queue;
    expect(queue.length).toBeGreaterThan(0);

    // Identity flip → wipe.
    await env.engine.resetReplica({ wipeMutations: true });
    expect((env.engine.mutations as unknown as { queue: unknown[] }).queue).toHaveLength(0);
  });

  test("a same-user 410 resync PRESERVES pending offline writes", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    await env.flush();

    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Note", { title: "draft" });
    await env.flush();
    const before = (env.engine.mutations as unknown as { queue: unknown[] }).queue.length;
    expect(before).toBeGreaterThan(0);

    // 410 RESYNC path is the default (wipeMutations omitted) — KEEP writes.
    await env.engine.resetReplica();
    expect(
      (env.engine.mutations as unknown as { queue: unknown[] }).queue.length,
    ).toBe(before);
  });

  // Row-revoked envelope: server pushes `row-revoked` to a
  // subscriber whose read policy was revoked for a specific row.
  // The engine must drop the row from the local replica and
  // tombstone at the carried seq so racing stale WS frames don't
  // resurrect it. Pins index.ts:792-806 + handleRowRevocation.
  test("row-revoked envelope removes the local row", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    env.server.insert("Note", { id: "n1", title: "visible" });
    await env.flush(50);
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();

    env.server.pushToUser("u1", {
      type: "row-revoked",
      entity: "Note",
      row_id: "n1",
      seq: env.server.nextSeqValue(),
    });
    await env.flush(50);
    expect(env.engine.store.get("Note", "n1")).toBeNull();

    // Stale insert at lower seq must not resurrect the row.
    env.server.pushToUser("u1", {
      seq: 1,
      entity: "Note",
      row_id: "n1",
      kind: "insert",
      data: { id: "n1", title: "STALE" },
      timestamp: "",
    });
    await env.flush();
    expect(env.engine.store.get("Note", "n1")).toBeNull();
  });

  // Optimistic mutation survives reconcile (pins #150 / index.ts
  // comment naming `hydrated_offline_mutations_survive_startup_reconcile`).
  // A pending mutation must NOT be tombstoned by reconcile's
  // "local row missing from server snapshot" branch — push() will
  // ship it; until then, the local row is the canonical state.
  test("pending mutation survives a sweeping reconcile", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();

    // Optimistic insert — push will queue this op, server returns
    // ops:[] from our mock (success no-op), but the queue still
    // tracks the in-flight mutation until clearance. Reconcile
    // running mid-flight sees `pendingRowKeys` and skips the row.
    const id = await env.engine.insert("Note", { title: "draft" });
    await env.flush();
    expect(env.engine.store.get("Note", id)).not.toBeNull();

    // Reconcile against a server that knows nothing about this row.
    await env.engine.reconcile(["Note"]);
    await env.flush();

    // The optimistic row survives because pendingRowKeys protected
    // it from the removal pass. Reverting #150 would tombstone it.
    expect(env.engine.store.get("Note", id)).not.toBeNull();
  });

  // Sign-out via the harness clears the resolved session AND the
  // replica. Pins the "tenant flips from X to null on signOut"
  // branch of refreshResolvedSession — resetReplica fires.
  test("signOut clears the resolved session and replica", async () => {
    env = createTestEnv({ visible: tenantScopedVisibility });
    env.server.seed("Recording", [{ id: "r1", orgId: "org-a" }]);
    env.signIn({ userId: "u1", tenantId: "org-a" });
    await env.start();
    await env.flush();
    expect(env.engine.store.list("Recording")).toHaveLength(1);
    expect(env.engine.resolvedSession().userId).toBe("u1");

    // signOut on the server side (revoke the token + broadcast),
    // then drop the local token. The engine refreshes its
    // session via the WS envelope; tenant flip X → null fires
    // resetReplica and a re-pull under the anonymous context.
    if (env.token) env.server.revoke(env.token);
    env.signOut();
    await env.engine.notifySessionChanged();
    await env.flush();

    expect(env.engine.resolvedSession().userId).toBeNull();
    expect(env.engine.resolvedSession().tenantId).toBeNull();
    expect(env.engine.store.list("Recording")).toHaveLength(0);
  });

  // Two visibility-change-style reconciles fired back-to-back must
  // produce EXACTLY ONE network round per entity. Stricter than
  // the previous "<= 2" bound, which would have passed even if
  // coalescing was broken. Pins the inFlightReconcile dedupe.
  test("back-to-back reconcile calls coalesce to a single fetch", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1" }]);
    await env.start();
    await env.flush();
    const before = env.transport.fetchCount();

    const p1 = env.engine.reconcile(["Note"]);
    const p2 = env.engine.reconcile(["Note"]);
    await Promise.all([p1, p2]);
    await env.flush();

    // Exactly one /api/entities/Note/cursor call should have fired.
    expect(env.transport.fetchCount() - before).toBe(1);
  });

  // Anonymous session pulls produce empty results under tenant-scoped
  // policy. Pins the "no session = no rows" baseline so future bugs
  // that accidentally leak public rows fail loudly.
  test("anonymous session sees nothing under tenant-scoped policy", async () => {
    env = createTestEnv({
      visible: (_e, rows, auth) =>
        auth.userId
          ? rows.filter((r) => (r as { orgId?: string }).orgId === auth.tenantId)
          : [],
    });
    env.server.seed("Recording", [{ id: "r1", orgId: "org-a" }]);
    // No signIn() — engine starts anonymous.
    await env.start();
    expect(env.engine.store.list("Recording")).toHaveLength(0);
  });

  // Token rotation preserves tenant (pins commit 8bb2b21c-style fix
  // for SessionStore::refresh). A user signs into org-a, then their
  // token is rotated (same user, fresh string). The resolved
  // session must still carry tenantId="org-a" — the rotation must
  // not silently downgrade to no-tenant.
  test("token rotation preserves the resolved tenant", async () => {
    env = createTestEnv({ visible: tenantScopedVisibility });
    env.server.seed("Recording", [{ id: "r1", orgId: "org-a" }]);
    env.signIn({ userId: "u1", tenantId: "org-a" });
    await env.start();
    await env.flush();
    expect(env.engine.resolvedSession().tenantId).toBe("org-a");

    // Mint a fresh token for the same user + tenant — the new
    // token replaces the old as the engine's currentToken.
    const fresh = env.server.signIn({ userId: "u1", tenantId: "org-a" });
    env.transport.setToken(fresh);
    await env.engine.notifySessionChanged();
    await env.flush();

    expect(env.engine.resolvedSession().userId).toBe("u1");
    expect(env.engine.resolvedSession().tenantId).toBe("org-a");
    // And the replica survives — token rotation must not wipe it.
    expect(env.engine.store.get("Recording", "r1")).not.toBeNull();
  });
});
