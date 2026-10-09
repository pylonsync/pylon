import { expect, test } from "bun:test";
import { LoroDoc, LoroText } from "loro-crdt";
import { SyncEngine } from "@pylonsync/sync";
import { LoroRegistry } from "./registry";
import { CRDT_FRAME_SNAPSHOT, encodeCrdtFrame } from "./wire";

test("a late follower requests full history after incremental frames", () => {
  const source = new LoroDoc();
  const text = source.getMap("row").setContainer("body", new LoroText());
  text.insert(0, "baseline");
  source.commit();
  const baseline = source.export({ mode: "snapshot" });
  const version = source.version();
  text.insert(text.length, " plus delta");
  source.commit();
  const delta = source.export({ mode: "update", from: version });
  const frame = (bytes: Uint8Array) => encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Doc", "row", bytes);
  const incomplete = new LoroRegistry();
  incomplete.applyBinaryFrame(frame(delta));
  expect(incomplete.doc("Doc", "row").toJSON()).not.toEqual(source.toJSON());

  const engine = new SyncEngine({ baseUrl: "http://test.invalid", multiTab: false, persist: false });
  const internal = engine as unknown as {
    dispatchBinaryFrame(bytes: Uint8Array): void;
    sendWs(message: unknown): void;
    broadcastToTabs(message: { type: string; bytes?: Uint8Array }): void;
    subscriptions: { handleForwardedRegister(message: Record<string, unknown>, tab: string): void };
  };
  const requests: unknown[] = [];
  internal.sendWs = (message) => { requests.push(message); };
  engine.subscribeCrdt("Doc", "row");
  internal.dispatchBinaryFrame(frame(baseline));
  internal.dispatchBinaryFrame(frame(delta));
  const late = new LoroRegistry();
  let forwarded = 0;
  internal.broadcastToTabs = (message) => {
    if (message.type === "binary") {
      forwarded++;
      late.applyBinaryFrame(message.bytes!);
    }
  };
  const requestCount = requests.length;
  const interest = { kind: "crdt", key: "Doc\x00row", entity: "Doc", rowId: "row" };
  internal.subscriptions.handleForwardedRegister(interest, "late-tab");
  expect(requests).toHaveLength(requestCount + 1);
  expect(requests.at(-1)).toEqual({ type: "crdt-subscribe", entity: "Doc", rowId: "row" });
  expect(forwarded).toBe(0);
  internal.dispatchBinaryFrame(frame(source.export({ mode: "snapshot" })));
  expect(late.doc("Doc", "row").toJSON()).toEqual(source.toJSON());
  internal.subscriptions.handleForwardedRegister(interest, "late-tab");
  expect(requests).toHaveLength(requestCount + 1);
});
