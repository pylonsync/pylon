import { afterAll, beforeAll, expect, test } from "bun:test";

import { connectShard } from "./connection";

const sockets: FakeWebSocket[] = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = 0;
  binaryType = "blob";
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(
    readonly url: string,
    readonly protocols?: string[],
  ) {
    sockets.push(this);
  }
  close() {
    this.readyState = 3;
    this.onclose?.();
  }
  send() {}
}

const realWebSocket = globalThis.WebSocket;
const realSetTimeout = globalThis.setTimeout;
const delays: number[] = [];
beforeAll(() => {
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
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

const settle = () => new Promise((r) => realSetTimeout(r, 0));

test("a ticket function gives each connection attempt a new ticket", async () => {
  let n = 0;
  const errors: string[] = [];
  const client = connectShard("zone", {
    subscriberId: "p1",
    baseUrl: "h",
    ticket: async () => {
      n += 1;
      if (n === 2) throw new Error("ticket service down");
      return `t${n}`;
    },
  });
  client.onError((e) => errors.push(e.message));
  await settle();
  expect(sockets[0].protocols).toEqual(["ticket.t1"]);

  // The socket drops; the next ticket fails, so it retries with backoff.
  sockets[0].close();
  await settle();
  await settle();
  expect(errors).toContain("ticket service down");
  expect(sockets.length).toBe(2);
  expect(sockets[1].protocols).toEqual(["ticket.t3"]);
  expect(delays).toEqual([500, 1000]);
  client.close();
});

test("send returns 0 and sends nothing while the socket is not open", () => {
  const errors: string[] = [];
  const client = connectShard("zone", { subscriberId: "p1", baseUrl: "h", ticket: "fixed" });
  client.onError((e) => errors.push(e.message));
  expect(sockets[sockets.length - 1].protocols).toEqual(["ticket.fixed"]);
  expect(client.send({ a: 1 })).toBe(0);
  expect(errors[0]).toContain("not open");
  client.close();
});

/** A version 2 frame with a JSON payload. */
function jsonFrame(kind: number, tick: number, body: unknown): ArrayBuffer {
  const payload = new TextEncoder().encode(JSON.stringify(body));
  const out = new Uint8Array(18 + payload.length);
  const view = new DataView(out.buffer);
  view.setUint8(0, kind);
  view.setUint8(1, 0);
  view.setUint32(6, tick);
  out.set(payload, 18);
  return out.buffer;
}

test("a transfer frame moves the client to the new shard with the ticket it carries", async () => {
  const asked: string[] = [];
  const client = connectShard<{ zone: string }>("west", {
    subscriberId: "p1",
    baseUrl: "h",
    ticket: (shard) => {
      asked.push(shard);
      return `own-${shard}`;
    },
  });
  const moves: Array<[string, string]> = [];
  const snaps: string[] = [];
  client.onTransfer((to, from) => moves.push([to, from]));
  client.onSnapshot((s) => snaps.push(s.zone));
  await settle();
  const first = sockets[sockets.length - 1];
  expect(first.url).toContain("shard=west");
  expect(first.protocols).toEqual(["ticket.own-west"]);
  first.readyState = 1;
  first.onopen?.();
  first.onmessage?.({ data: jsonFrame(1, 500, { zone: "west" }) });
  expect(client.tick).toBe(500);

  // The server moves the player, then closes.
  first.onmessage?.({ data: jsonFrame(4, 501, { shard: "east", ticket: "from-server" }) });
  expect(client.shardId).toBe("east");
  expect(moves).toEqual([["east", "west"]]);
  expect(client.tick).toBe(-1);
  first.close();
  await settle();

  // At once, to the new shard, with the ticket from the frame.
  const second = sockets[sockets.length - 1];
  expect(second).not.toBe(first);
  expect(second.url).toContain("shard=east");
  expect(second.protocols).toEqual(["ticket.from-server"]);
  expect(delays[delays.length - 1]).toBe(0);
  second.readyState = 1;
  second.onopen?.();
  // The new shard's ticks start low; the client takes them.
  second.onmessage?.({ data: jsonFrame(1, 3, { zone: "east" }) });
  expect(client.tick).toBe(3);
  expect(snaps).toEqual(["west", "east"]);

  // A later reconnect asks the ticket function for the new shard.
  second.close();
  await settle();
  expect(asked).toEqual(["west", "east"]);
  expect(sockets[sockets.length - 1].protocols).toEqual(["ticket.own-east"]);
  client.close();
});

test("an explicit wsUrl follows a transfer", async () => {
  const client = connectShard("west", {
    subscriberId: "p1",
    wsUrl: "ws://h/shard?shard=west&sid=p1&v=2",
  });
  await settle();
  const first = sockets[sockets.length - 1];
  first.readyState = 1;
  first.onopen?.();
  first.onmessage?.({ data: jsonFrame(4, 9, { shard: "east", ticket: "t" }) });
  first.close();
  await settle();
  expect(sockets[sockets.length - 1].url).toBe("ws://h/shard?shard=east&sid=p1&v=2");
  client.close();
});

test("after a transfer, retries keep the transfer ticket until the new shard sends a frame", async () => {
  const client = connectShard("west", {
    subscriberId: "p1",
    baseUrl: "h",
    ticket: (shard) => `own-${shard}`,
  });
  await settle();
  const first = sockets[sockets.length - 1];
  first.readyState = 1;
  first.onopen?.();
  first.onmessage?.({ data: jsonFrame(4, 1, { shard: "east", ticket: "from-server" }) });
  first.close();
  await settle();
  // The first attempt at east fails before opening: the retry still uses
  // the server's ticket.
  const failed = sockets[sockets.length - 1];
  expect(failed.protocols).toEqual(["ticket.from-server"]);
  failed.close();
  await settle();
  // Opened, then closed before any frame (the shard refused the ticket, say):
  // still the server's ticket.
  const opened = sockets[sockets.length - 1];
  expect(opened.protocols).toEqual(["ticket.from-server"]);
  opened.readyState = 1;
  opened.onopen?.();
  opened.close();
  await settle();
  const answered = sockets[sockets.length - 1];
  expect(answered.protocols).toEqual(["ticket.from-server"]);
  // A frame from the new shard: the ticket function takes over.
  answered.readyState = 1;
  answered.onopen?.();
  answered.onmessage?.({ data: jsonFrame(1, 2, { zone: "east" }) });
  answered.close();
  await settle();
  expect(sockets[sockets.length - 1].protocols).toEqual(["ticket.own-east"]);
  client.close();
});

test("a fixed ticket names the first shard and is not sent after a transfer", async () => {
  const client = connectShard("west", { subscriberId: "p1", baseUrl: "h", ticket: "for-west" });
  await settle();
  const first = sockets[sockets.length - 1];
  expect(first.protocols).toEqual(["ticket.for-west"]);
  first.readyState = 1;
  first.onopen?.();
  first.onmessage?.({ data: jsonFrame(4, 1, { shard: "east", ticket: "for-east" }) });
  first.close();
  await settle();
  const second = sockets[sockets.length - 1];
  second.readyState = 1;
  second.onopen?.();
  second.close();
  await settle();
  expect(sockets[sockets.length - 1].protocols).toEqual(["ticket.for-east"]);
  client.close();
});
