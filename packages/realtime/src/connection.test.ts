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
