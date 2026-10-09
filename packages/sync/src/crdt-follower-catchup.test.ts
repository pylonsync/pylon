// A new follower needs a full authorized snapshot from the server.
// Binary update frames cannot serve as a complete document cache.
import { describe, expect, test } from "bun:test";
import { SubscriptionCoordinator, crdtKey } from "./subscription-coordinator";
import { ServerSubscriptions } from "./server-subscriptions";
import { crdtFrameKey, SyncEngine } from "./index";

function encodeSnapshotFrame(
  type: number,
  entity: string,
  rowId: string,
  payload: Uint8Array,
): Uint8Array {
  const enc = new TextEncoder();
  const e = enc.encode(entity);
  const r = enc.encode(rowId);
  const out = new Uint8Array(1 + 2 + e.length + 2 + r.length + payload.length);
  const view = new DataView(out.buffer);
  out[0] = type;
  view.setUint16(1, e.length, false);
  out.set(e, 3);
  view.setUint16(3 + e.length, r.length, false);
  out.set(r, 5 + e.length);
  out.set(payload, 5 + e.length + r.length);
  return out;
}

describe("crdtFrameKey", () => {
  test("keys a snapshot frame by entity|rowId", () => {
    const frame = encodeSnapshotFrame(0x10, "Doc", "row1", new Uint8Array(3));
    expect(crdtFrameKey(frame)).toBe("Doc|row1");
  });

  test("ignores update frames and garbage", () => {
    const update = encodeSnapshotFrame(0x11, "Doc", "row1", new Uint8Array(3));
    expect(crdtFrameKey(update)).toBeNull();
    expect(crdtFrameKey(new Uint8Array([0x10, 0, 9]))).toBeNull();
    expect(crdtFrameKey(new Uint8Array(0))).toBeNull();
  });
});

describe("forwarded register replay", () => {
  function harness() {
    const sent: unknown[] = [];
    const serverSubs = new ServerSubscriptions((msg) => {
      sent.push(msg);
      return true;
    });
    const coordinator = new SubscriptionCoordinator(serverSubs, {
      isLeader: () => true,
      broadcastToTabs: () => {},
    });
    return { coordinator, sent };
  }

  test("an existing subscription requests a full snapshot for a new follower", () => {
    const { coordinator, sent } = harness();
    // Leader's own component holds the row → WS sub is alive.
    coordinator.subscribeCrdt("Doc", "row1");
    // A second tab needs a fresh snapshot even though the row is subscribed.
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-2",
    );
    expect(sent.filter((message) => (message as { type?: string }).type === "crdt-subscribe")).toHaveLength(2);
  });

  test("a fresh subscription needs only its initial request", () => {
    const { coordinator, sent } = harness();
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row9"), entity: "Doc", rowId: "row9" },
      "tab-2",
    );
    // First interest in the row → real crdt-subscribe goes out; the
    // server's own catch-up snapshot will be fanned to tabs.
    expect(sent).toHaveLength(1);
    expect(
      sent.some(
        (m) => (m as { type?: string }).type === "crdt-subscribe",
      ),
    ).toBe(true);
  });

  test("a second follower refreshes state but duplicate registration does not", () => {
    const { coordinator, sent } = harness();
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-2",
    );
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-3",
    );
    expect(sent.filter((message) => (message as { type?: string }).type === "crdt-subscribe")).toHaveLength(2);
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" }, "tab-3",
    );
    expect(sent).toHaveLength(2);
  });
});


test("binary fanout follows row subscriptions and retains snapshot catch-up", () => {
  const engine = new SyncEngine({ baseUrl: "http://test.invalid", multiTab: false, persist: false });
  const internal = engine as unknown as {
    subscriptions: SubscriptionCoordinator;
    dispatchBinaryFrame(bytes: Uint8Array): void;
    broadcastToTabs(message: { type: string; bytes?: Uint8Array }): void;
  };
  const sent: Uint8Array[] = [];
  internal.broadcastToTabs = (message) => { if (message.type === "binary") sent.push(message.bytes!); };
  const subscription = { kind: "crdt" as const, key: crdtKey("Doc", "wanted"), entity: "Doc", rowId: "wanted" };
  internal.subscriptions.handleForwardedRegister(subscription, "follower");
  const payload = new Uint8Array(1024 * 1024);
  for (let i = 0; i < 10; i++) {
    internal.dispatchBinaryFrame(encodeSnapshotFrame(i % 2 ? 0x10 : 0x11, "Doc", `other-${i}`, payload));
  }
  expect(sent).toHaveLength(0);
  const snapshot = encodeSnapshotFrame(0x10, "Doc", "wanted", payload);
  const update = encodeSnapshotFrame(0x11, "Doc", "wanted", new Uint8Array([1]));
  internal.dispatchBinaryFrame(snapshot);
  internal.dispatchBinaryFrame(update);
  expect(sent).toEqual([snapshot, update]);
  internal.subscriptions.handleForwardedRegister(subscription, "late-follower");
  expect(sent).toHaveLength(2); // Never replay cached binary data to a new follower.
  for (const bytes of [new Uint8Array([0x20]), new Uint8Array([0x10, 0, 9]), new Uint8Array([0x10, 0, 1, 0xff, 0, 1, 0x61])]) {
    internal.dispatchBinaryFrame(bytes);
    expect(sent.at(-1)).toBe(bytes);
  }
  for (const follower of ["follower", "late-follower"]) {
    internal.subscriptions.handleForwardedUnregister({ type: "sub-unregister", ...subscription }, follower);
  }
  const count = sent.length;
  internal.dispatchBinaryFrame(snapshot);
  internal.dispatchBinaryFrame(update);
  internal.dispatchBinaryFrame(new Uint8Array([0x20]));
  expect(sent).toHaveLength(count);
});
