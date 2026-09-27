import { afterAll, beforeAll, beforeEach, expect, test } from "bun:test";

import { connectShard } from "./connection";
import fixtures from "./replication.fixtures.json";
import type { ReplicationSummary } from "./replication";

/** A WebTransport session the test plays the server of. */
class FakeWebTransport {
  static sessions: FakeWebTransport[] = [];
  /** Set to make the next session's `ready` reject, or never settle. */
  static nextReady: "open" | "fail" | "hang" = "open";

  readonly ready: Promise<void>;
  readonly closed: Promise<{ closeCode?: number; reason?: string }>;
  /** Messages the client wrote on its stream, split at the length prefixes. */
  readonly streamIn: Uint8Array[] = [];
  /** Datagrams the client sent. */
  readonly datagramsIn: Uint8Array[] = [];
  clientClose: { closeCode?: number; reason?: string } | null = null;
  private toClient!: ReadableStreamDefaultController<Uint8Array>;
  private datagramsToClient!: ReadableStreamDefaultController<Uint8Array>;
  private settleClosed!: (info: { closeCode?: number; reason?: string }) => void;
  private streamBytes: number[] = [];

  readonly datagrams: {
    readable: ReadableStream<Uint8Array>;
    createWritable: () => WritableStream<Uint8Array>;
  };

  constructor(
    readonly url: string,
    readonly options: {
      serverCertificateHashes?: Array<{ algorithm: string; value: Uint8Array }>;
      requireUnreliable?: boolean;
    },
  ) {
    FakeWebTransport.sessions.push(this);
    const mode = FakeWebTransport.nextReady;
    FakeWebTransport.nextReady = "open";
    let failReady: (e: Error) => void = () => {};
    this.ready = new Promise((resolve, reject) => {
      failReady = reject;
      if (mode === "open") resolve();
      if (mode === "fail") reject(new Error("QUIC handshake failed"));
    });
    this.closed = new Promise((resolve, reject) => {
      this.settleClosed = resolve;
      if (mode === "fail") this.ready.catch(reject);
    });
    void failReady;
    this.datagrams = {
      readable: new ReadableStream({ start: (c) => void (this.datagramsToClient = c) }),
      createWritable: () =>
        new WritableStream({ write: (chunk) => void this.datagramsIn.push(chunk) }),
    };
  }

  async createBidirectionalStream() {
    return {
      readable: new ReadableStream<Uint8Array>({ start: (c) => void (this.toClient = c) }),
      writable: new WritableStream<Uint8Array>({
        write: (chunk) => {
          this.streamBytes.push(...chunk);
          while (this.streamBytes.length >= 4) {
            const len = new DataView(Uint8Array.from(this.streamBytes.slice(0, 4)).buffer).getUint32(0);
            if (this.streamBytes.length < 4 + len) break;
            this.streamIn.push(Uint8Array.from(this.streamBytes.slice(4, 4 + len)));
            this.streamBytes = this.streamBytes.slice(4 + len);
          }
        },
      }),
    };
  }

  close(info?: { closeCode?: number; reason?: string }) {
    this.clientClose = info ?? {};
    this.settleClosed({ closeCode: info?.closeCode ?? 0, reason: info?.reason ?? "" });
  }

  /** The hello the client sent. */
  get hello(): Record<string, string> {
    return JSON.parse(new TextDecoder().decode(this.streamIn[0]));
  }

  /** Send frames on the stream, each length-prefixed, in one chunk per `split` bytes. */
  sendFrames(frames: Uint8Array[], split = 5) {
    const all: number[] = [];
    for (const f of frames) {
      const len = new Uint8Array(4);
      new DataView(len.buffer).setUint32(0, f.length);
      all.push(...len, ...f);
    }
    for (let i = 0; i < all.length; i += split) {
      this.toClient.enqueue(Uint8Array.from(all.slice(i, i + split)));
    }
  }

  sendDatagram(d: Uint8Array) {
    this.datagramsToClient.enqueue(d);
  }

  serverClose(closeCode: number, reason: string) {
    this.settleClosed({ closeCode, reason });
  }
}

const sockets: FakeWebSocket[] = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = 0;
  binaryType = "blob";
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((e?: { code: number; reason: string }) => void) | null = null;
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

const HASH = new Uint8Array(32).fill(7);
let infoStatus = 200;
const fetched: string[] = [];

const realWebSocket = globalThis.WebSocket;
const realFetch = globalThis.fetch;
const g = globalThis as { WebTransport?: unknown };
beforeAll(() => {
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
  globalThis.fetch = (async (url: string) => {
    fetched.push(String(url));
    if (infoStatus !== 200) return new Response("{}", { status: infoStatus });
    return new Response(
      JSON.stringify({ url: "https://wt.example:443/shard", certHashes: [btoa(String.fromCharCode(...HASH))] }),
    );
  }) as unknown as typeof fetch;
});
afterAll(() => {
  globalThis.WebSocket = realWebSocket;
  globalThis.fetch = realFetch;
  delete g.WebTransport;
});
beforeEach(() => {
  g.WebTransport = FakeWebTransport;
  FakeWebTransport.sessions.length = 0;
  sockets.length = 0;
  fetched.length = 0;
  infoStatus = 200;
});

async function until(what: string, cond: () => boolean) {
  for (let i = 0; i < 200; i++) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 5));
  }
  throw new Error(`timed out waiting for ${what}`);
}

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** A version 2 frame: kind, codec, tick, ack, payload. */
function frame(kind: number, codec: number, tick: number, ack: number, payload: Uint8Array): Uint8Array {
  const f = new Uint8Array(18 + payload.length);
  const v = new DataView(f.buffer);
  f[0] = kind;
  f[1] = codec;
  v.setUint32(6, tick);
  v.setUint32(14, ack);
  f.set(payload, 18);
  return f;
}

const json = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));

// Rust-encoded: a full frame with entities 1, 2, 3, then a datagram for
// frame 2 (tick 3, ack 30) and one for frame 1 (tick 2, ack 20).
const fullFrame = bytes(fixtures.datagrams[0].stream as string);
const datagram2 = fixtures.datagrams[1] as { datagram: string; tick: number; updated: number[] };
const datagram1 = fixtures.datagrams[2] as { datagram: string; tick: number };

test("a WebTransport session: hello, stream frames, datagrams with acks, and inputs", async () => {
  const updates: Array<[ReplicationSummary, number, number]> = [];
  const client = connectShard("field", {
    subscriberId: "p1",
    baseUrl: "app.example",
    ticket: "t1",
    transport: "webtransport",
  });
  client.onReplication((_, summary, tick, ack) => updates.push([summary, tick, ack]));
  await until("the session", () => client.connected);
  expect(client.transport).toBe("webtransport");
  expect(fetched).toEqual(["http://app.example/_pylon/shard/webtransport"]);

  const wt = FakeWebTransport.sessions[0];
  expect(wt.url).toBe("https://wt.example:443/shard");
  expect(wt.options.requireUnreliable).toBe(true);
  expect(wt.options.serverCertificateHashes).toEqual([{ algorithm: "sha-256", value: HASH }]);
  await until("the hello", () => wt.streamIn.length === 1);
  expect(wt.hello).toEqual({ shard: "field", sid: "p1", ticket: "t1" });

  // The baseline on the stream, split across chunks.
  wt.sendFrames([frame(3, 4, 1, 0, fullFrame)], 3);
  await until("the full frame", () => client.entities.size === 3);
  expect(updates[0][0].full).toBe(true);
  expect(updates[0][1]).toBe(1);

  // An update datagram: applied, then acked with the frames applied.
  wt.sendDatagram(bytes(datagram2.datagram));
  await until("the ack", () => wt.datagramsIn.length === 1);
  expect([...wt.datagramsIn[0]]).toEqual([1, 1, 2, 1]);
  expect(updates[1][0]).toEqual({ full: false, spawned: [], updated: datagram2.updated, despawned: [] });
  expect(updates[1][1]).toBe(datagram2.tick);
  expect(client.tick).toBe(3);
  expect(client.ack).toBe(30);
  expect(client.entities.get(1)?.qx).toBe(175);

  // An older tick's datagram, late: dropped unacked, as if lost.
  wt.sendDatagram(bytes(datagram1.datagram));
  wt.sendFrames([frame(1, 0, 4, 30, json({ n: 1 }))]);
  await until("the snapshot", () => client.tick === 4);
  expect(wt.datagramsIn.length).toBe(1);
  expect(updates.length).toBe(2);

  // Inputs go on the stream after a type byte: JSON until the shard's
  // codec is known (the snapshot said JSON).
  expect(client.send({ move: 1 })).toBe(1);
  await until("the input", () => wt.streamIn.length === 2);
  expect(wt.streamIn[1][0]).toBe(2);
  expect(JSON.parse(new TextDecoder().decode(wt.streamIn[1].subarray(1)))).toEqual({
    input: { move: 1 },
    client_seq: 1,
  });

  client.close();
  await until("the close", () => wt.clientClose !== null);
  expect(wt.clientClose).toEqual({ closeCode: 0, reason: "" });
  expect(client.connected).toBe(false);
});

test("a WebTransport refusal of fixed credentials stops the client", async () => {
  const errors: string[] = [];
  const client = connectShard("field", {
    subscriberId: "p1",
    baseUrl: "h",
    ticket: "expired",
    transport: "webtransport",
    reconnectBackoffMs: 1,
  });
  client.onError((e) => errors.push(e.message));
  await until("the session", () => client.connected);
  FakeWebTransport.sessions[0].serverClose(1, "unauthorized: ticket expired");
  await until("the refusal", () => errors.length === 1);
  expect(errors[0]).toContain("refused the connection (unauthorized: ticket expired)");
  await new Promise((r) => setTimeout(r, 30));
  expect(FakeWebTransport.sessions.length).toBe(1);
  client.close();
});

test("a transfer over WebTransport reconnects with the ticket it carries", async () => {
  const client = connectShard("west", {
    subscriberId: "p1",
    baseUrl: "h",
    ticket: "t-west",
    transport: "webtransport",
  });
  await until("the session", () => client.connected);
  const first = FakeWebTransport.sessions[0];
  first.sendFrames([frame(4, 0, 9, 0, json({ shard: "east", ticket: "t-east" }))]);
  await until("the transfer", () => client.shardId === "east");
  first.serverClose(0, "moved to shard east");
  await until("the second session", () => FakeWebTransport.sessions.length === 2 && client.connected);
  const second = FakeWebTransport.sessions[1];
  await until("the hello", () => second.streamIn.length === 1);
  expect(second.hello).toEqual({ shard: "east", sid: "p1", ticket: "t-east" });
  client.close();
});

test("auto uses a WebSocket, for good, when the app does not serve WebTransport", async () => {
  infoStatus = 404;
  const client = connectShard("field", {
    subscriberId: "p1",
    baseUrl: "h",
    transport: "auto",
    reconnectBackoffMs: 1,
  });
  await until("the socket", () => sockets.length === 1);
  expect(FakeWebTransport.sessions.length).toBe(0);
  sockets[0].readyState = 1;
  sockets[0].onopen?.();
  expect(client.transport).toBe("websocket");
  sockets[0].close();
  await until("the reconnect", () => sockets.length === 2);
  expect(fetched.length).toBe(1);
  client.close();
});

test("auto falls back to a WebSocket when the session fails or does not open in time", async () => {
  FakeWebTransport.nextReady = "fail";
  const a = connectShard("field", { subscriberId: "p1", baseUrl: "h", transport: "auto" });
  await until("the socket", () => sockets.length === 1);
  expect(FakeWebTransport.sessions.length).toBe(1);
  a.close();

  FakeWebTransport.nextReady = "hang";
  const b = connectShard("field", {
    subscriberId: "p1",
    baseUrl: "h",
    transport: "auto",
    webTransportTimeoutMs: 20,
  });
  await until("the socket", () => sockets.length === 2);
  expect(FakeWebTransport.sessions[1].clientClose).not.toBeNull();
  b.close();
});

test("auto goes straight to a WebSocket without the browser API", () => {
  delete g.WebTransport;
  const client = connectShard("field", { subscriberId: "p1", baseUrl: "h", transport: "auto" });
  expect(sockets.length).toBe(1);
  expect(fetched.length).toBe(0);
  client.close();
});

test("a forced WebTransport stops with an error when the app does not serve it", async () => {
  infoStatus = 404;
  const errors: string[] = [];
  const client = connectShard("field", {
    subscriberId: "p1",
    baseUrl: "h",
    transport: "webtransport",
    reconnectBackoffMs: 1,
  });
  client.onError((e) => errors.push(e.message));
  await until("the error", () => errors.length === 1);
  expect(errors[0]).toContain("the app does not serve WebTransport");
  await new Promise((r) => setTimeout(r, 30));
  expect(fetched.length).toBe(1);
  expect(sockets.length).toBe(0);
  client.close();
});
