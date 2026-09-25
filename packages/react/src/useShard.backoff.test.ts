import { afterAll, beforeAll, expect, test } from "bun:test";

import { SHARD_HEADER_LEN, ShardCodec, ShardFrameKind } from "@pylonsync/realtime";
import { connectShard } from "./useShard";

// A server that opens the socket and sends a replication frame the client
// can never apply (version 2). Each close schedules a reconnect; the
// delays must grow instead of resetting on every open.
const sockets: FakeWebSocket[] = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = 0;
  binaryType = "blob";
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor() {
    sockets.push(this);
  }
  close() {
    this.readyState = 3;
    this.onclose?.();
  }
  send() {}
}

function replicationFrame(version: number): ArrayBuffer {
  const buf = new ArrayBuffer(SHARD_HEADER_LEN + 9);
  const view = new DataView(buf);
  view.setUint8(0, ShardFrameKind.Replication);
  view.setUint8(1, ShardCodec.Replication);
  // FULL, precision 0.01, no despawns, spawns, or updates.
  new Uint8Array(buf, SHARD_HEADER_LEN).set([version, 1, 0x0a, 0xd7, 0x23, 0x3c, 0, 0, 0]);
  return buf;
}

/** Version 2: the client can never apply it. */
const badReplicationFrame = () => replicationFrame(2);

const realWebSocket = globalThis.WebSocket;
const realSetTimeout = globalThis.setTimeout;
const delays: number[] = [];
beforeAll(() => {
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
  // Record reconnect delays and run the reconnect at once.
  globalThis.setTimeout = ((fn: () => void, ms: number) => {
    delays.push(ms);
    queueMicrotask(fn);
    return 0;
  }) as unknown as typeof setTimeout;
});
afterAll(() => {
  globalThis.WebSocket = realWebSocket;
  globalThis.setTimeout = realSetTimeout;
});

test("a frame that always fails backs off instead of reconnecting at the shortest delay", async () => {
  const errors: Error[] = [];
  const client = connectShard("zone", { subscriberId: "p1", baseUrl: "h" });
  client.onError((e) => errors.push(e));
  for (let i = 0; i < 5; i++) {
    const ws = sockets[i];
    ws.readyState = 1;
    ws.onopen?.();
    ws.onmessage?.({ data: badReplicationFrame() });
    await new Promise((r) => realSetTimeout(r, 0));
  }
  client.close();
  expect(delays.slice(0, 5)).toEqual([500, 1000, 2000, 4000, 8000]);
  expect(errors.some((e) => e.message.includes("version 2"))).toBe(true);
});

test("a frame that applies resets the backoff", async () => {
  sockets.length = 0;
  delays.length = 0;
  const client = connectShard("zone", { subscriberId: "p1", baseUrl: "h" });
  for (let i = 0; i < 3; i++) {
    const ws = sockets[i];
    ws.readyState = 1;
    ws.onopen?.();
    ws.onmessage?.({ data: badReplicationFrame() });
    await new Promise((r) => realSetTimeout(r, 0));
  }
  const ws = sockets[3];
  ws.readyState = 1;
  ws.onopen?.();
  ws.onmessage?.({ data: replicationFrame(1) });
  ws.close();
  await new Promise((r) => realSetTimeout(r, 0));
  client.close();
  expect(delays).toEqual([500, 1000, 2000, 500]);
});
