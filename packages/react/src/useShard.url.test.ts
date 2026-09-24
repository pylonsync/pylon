import { afterAll, afterEach, beforeAll, expect, test } from "bun:test";

import { connectShard } from "./useShard";

// Record the URL and subprotocols connectShard opens, without a network.
const opened: Array<{ url: string; protocols?: string[] }> = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = 0;
  binaryType = "blob";
  onopen: unknown = null;
  onmessage: unknown = null;
  onerror: unknown = null;
  onclose: unknown = null;
  constructor(url: string, protocols?: string[]) {
    opened.push({ url, protocols });
  }
  close() {}
  send() {}
}
const realWebSocket = globalThis.WebSocket;
beforeAll(() => {
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
});
afterAll(() => {
  globalThis.WebSocket = realWebSocket;
});
afterEach(() => {
  opened.length = 0;
});

test("the default URL is /shard on the server origin", () => {
  connectShard("zone-3", { subscriberId: "p1", baseUrl: "game.example.com", autoReconnect: false }).close();
  expect(opened[0].url).toBe("ws://game.example.com/shard?shard=zone-3&sid=p1&v=2");
});

test("wsPort selects the dedicated shard port", () => {
  connectShard("zone-3", {
    subscriberId: "p1",
    baseUrl: "game.example.com:4321",
    wsPort: 4324,
    autoReconnect: false,
  }).close();
  expect(opened[0].url).toBe("ws://game.example.com:4324/?shard=zone-3&sid=p1&v=2");
});

test("the token and ticket ride as subprotocols", () => {
  connectShard("zone-3", {
    subscriberId: "c 12",
    baseUrl: "h",
    token: "t/1",
    ticket: "v1.a.b",
    autoReconnect: false,
  }).close();
  expect(opened[0].protocols).toEqual(["bearer.t%2F1", "ticket.v1.a.b"]);
  expect(opened[0].url).toContain("sid=c+12");
});
