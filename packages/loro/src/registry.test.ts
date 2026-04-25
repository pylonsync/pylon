// ---------------------------------------------------------------------------
// Registry tests. Exercise the doc cache + binary frame routing
// in isolation from the WebSocket / SyncEngine plumbing.
// ---------------------------------------------------------------------------

import { test, expect } from "bun:test";
import { LoroDoc } from "loro-crdt";
import { LoroRegistry } from "./registry";
import { CRDT_FRAME_SNAPSHOT, encodeCrdtFrame } from "./wire";

test("doc() returns the same instance for the same row across calls", () => {
  const reg = new LoroRegistry();
  const a = reg.doc("Note", "n1");
  const b = reg.doc("Note", "n1");
  expect(a).toBe(b); // referential identity, not just equal
});

test("distinct (entity, rowId) pairs get distinct docs", () => {
  const reg = new LoroRegistry();
  const a = reg.doc("Note", "n1");
  const b = reg.doc("Note", "n2");
  const c = reg.doc("Other", "n1");
  expect(a).not.toBe(b);
  expect(a).not.toBe(c);
  expect(b).not.toBe(c);
  expect(reg.cachedRows()).toBe(3);
});

test("applyBinaryFrame imports a server snapshot into the right doc", () => {
  // Build a Loro snapshot the way the server would.
  const upstream = new LoroDoc();
  upstream.getText("body").insert(0, "hello");
  upstream.commit();
  const snap = upstream.export({ mode: "snapshot" });
  const frame = encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Note", "n1", snap);

  const reg = new LoroRegistry();
  const ok = reg.applyBinaryFrame(frame);
  expect(ok).toBe(true);

  const local = reg.doc("Note", "n1");
  expect(local.getText("body").toString()).toBe("hello");
});

test("subscribe() fires on every applied frame", () => {
  const reg = new LoroRegistry();
  let calls = 0;
  const unsub = reg.subscribe("Note", "n1", () => {
    calls += 1;
  });

  const upstream = new LoroDoc();
  upstream.getText("body").insert(0, "first");
  upstream.commit();
  reg.applyBinaryFrame(
    encodeCrdtFrame(
      CRDT_FRAME_SNAPSHOT,
      "Note",
      "n1",
      upstream.export({ mode: "snapshot" }),
    ),
  );

  upstream.getText("body").insert(5, " update");
  upstream.commit();
  reg.applyBinaryFrame(
    encodeCrdtFrame(
      CRDT_FRAME_SNAPSHOT,
      "Note",
      "n1",
      upstream.export({ mode: "snapshot" }),
    ),
  );

  expect(calls).toBe(2);
  unsub();

  // After unsub, future frames don't fire the listener.
  upstream.getText("body").insert(0, "post-unsub ");
  upstream.commit();
  reg.applyBinaryFrame(
    encodeCrdtFrame(
      CRDT_FRAME_SNAPSHOT,
      "Note",
      "n1",
      upstream.export({ mode: "snapshot" }),
    ),
  );
  expect(calls).toBe(2);
});

test("subscribe() listener is per-row — sibling rows don't fire it", () => {
  const reg = new LoroRegistry();
  let n1Calls = 0;
  let n2Calls = 0;
  reg.subscribe("Note", "n1", () => (n1Calls += 1));
  reg.subscribe("Note", "n2", () => (n2Calls += 1));

  const doc = new LoroDoc();
  doc.getText("body").insert(0, "x");
  doc.commit();
  const snap = doc.export({ mode: "snapshot" });

  reg.applyBinaryFrame(encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Note", "n1", snap));
  expect(n1Calls).toBe(1);
  expect(n2Calls).toBe(0);

  reg.applyBinaryFrame(encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Note", "n2", snap));
  expect(n1Calls).toBe(1);
  expect(n2Calls).toBe(1);
});

test("malformed binary frame returns false without crashing", () => {
  const reg = new LoroRegistry();
  expect(reg.applyBinaryFrame(new Uint8Array([0x10, 0x00]))).toBe(false);
});

test("unknown frame type returns false", () => {
  const reg = new LoroRegistry();
  // Type byte 0xFF isn't a recognized snapshot/update marker.
  const frame = encodeCrdtFrame(0xff, "Note", "n1", new Uint8Array(0));
  expect(reg.applyBinaryFrame(frame)).toBe(false);
});

test("two registries hydrated from snapshots converge after exchange", () => {
  // End-to-end "two browser tabs" simulation. Each tab has its own
  // registry; they exchange snapshots and converge to the same state.
  const a = new LoroRegistry();
  const b = new LoroRegistry();

  // Tab A locally writes "from-a".
  a.doc("Note", "n1").getText("body").insert(0, "from-a");
  a.doc("Note", "n1").commit();
  // Tab B locally writes "from-b".
  b.doc("Note", "n1").getText("body").insert(0, "from-b");
  b.doc("Note", "n1").commit();

  // Each tab broadcasts its snapshot to the other.
  const snapA = a.doc("Note", "n1").export({ mode: "snapshot" });
  const snapB = b.doc("Note", "n1").export({ mode: "snapshot" });

  a.applyBinaryFrame(encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Note", "n1", snapB));
  b.applyBinaryFrame(encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Note", "n1", snapA));

  // Both registries' docs converge — the result contains BOTH writes'
  // characters in some deterministic Loro-merged order. The exact
  // output isn't pinned (Loro picks one) but both replicas agree.
  const aText = a.doc("Note", "n1").getText("body").toString();
  const bText = b.doc("Note", "n1").getText("body").toString();
  expect(aText).toBe(bText);
  expect(aText.length).toBeGreaterThan(0);
});
