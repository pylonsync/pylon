// Follower catch-up for CRDT rows. The server ships its catch-up
// snapshot only on a FRESH `crdt-subscribe`; when a follower tab
// registers interest in a row the leader (or another follower) already
// subscribed, no wire traffic happens — pre-fix the new tab's LoroDoc
// stayed empty until the next live edit ("open the doc in a second tab
// and the content doesn't render"). The leader now caches the latest
// snapshot frame per row and replays it over the tab channel.
import { describe, expect, test } from "bun:test";
import { SubscriptionCoordinator, crdtKey } from "./subscription-coordinator";
import { ServerSubscriptions } from "./server-subscriptions";
import { crdtFrameKey } from "./index";

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
    const replayed: Array<[string, string]> = [];
    const serverSubs = new ServerSubscriptions((msg) => {
      sent.push(msg);
      return true;
    });
    const coordinator = new SubscriptionCoordinator(serverSubs, {
      isLeader: () => true,
      broadcastToTabs: () => {},
      replayCrdtFrame: (entity, rowId) => {
        replayed.push([entity, rowId]);
      },
    });
    return { coordinator, sent, replayed };
  }

  test("already-alive subscription replays the cached snapshot to followers", () => {
    const { coordinator, replayed } = harness();
    // Leader's own component holds the row → WS sub is alive.
    coordinator.subscribeCrdt("Doc", "row1");
    // A second tab opens the same doc and forwards its interest. No new
    // crdt-subscribe goes out (the sub exists), so the ONLY way this
    // tab gets state is the replay.
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-2",
    );
    expect(replayed).toEqual([["Doc", "row1"]]);
  });

  test("fresh subscription does NOT replay — the server catch-up covers it", () => {
    const { coordinator, replayed, sent } = harness();
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row9"), entity: "Doc", rowId: "row9" },
      "tab-2",
    );
    // First interest in the row → real crdt-subscribe goes out; the
    // server's own catch-up snapshot will be fanned to tabs.
    expect(replayed).toEqual([]);
    expect(
      sent.some(
        (m) => (m as { type?: string }).type === "crdt-subscribe",
      ),
    ).toBe(true);
  });

  test("second follower for the same row also gets a replay", () => {
    const { coordinator, replayed } = harness();
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-2",
    );
    coordinator.handleForwardedRegister(
      { kind: "crdt", key: crdtKey("Doc", "row1"), entity: "Doc", rowId: "row1" },
      "tab-3",
    );
    expect(replayed).toEqual([["Doc", "row1"]]);
  });
});
