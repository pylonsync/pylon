import { afterAll, beforeAll, expect, test } from "bun:test";

import { replicationFrame, shardFrame } from "../test/frames";
import { connectShardGame } from "./game";
import { ShardCodec, ShardFrameKind } from "./wire";

const sockets: FakeWebSocket[] = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = 0;
  binaryType = "blob";
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(readonly url: string) {
    sockets.push(this);
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: ArrayBuffer) {
    this.onmessage?.({ data: frame });
  }
  close() {
    this.readyState = 3;
    this.onclose?.();
  }
  send(data: string) {
    this.sent.push(data);
  }
}

const realWebSocket = globalThis.WebSocket;
const realSetTimeout = globalThis.setTimeout;
beforeAll(() => {
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
  globalThis.setTimeout = ((fn: () => void) => {
    queueMicrotask(fn);
    return 0;
  }) as unknown as typeof setTimeout;
});
afterAll(() => {
  globalThis.WebSocket = realWebSocket;
  globalThis.setTimeout = realSetTimeout;
});

function rejection(tick: number, clientSeq: number): ArrayBuffer {
  const body = new TextEncoder().encode(
    JSON.stringify({ client_seq: clientSeq, code: "invalid", message: "no" }),
  );
  return shardFrame(ShardFrameKind.InputRejected, ShardCodec.Json, tick, 0, body);
}

test("a render loop draws entities between frames, and prediction follows acks", async () => {
  let time = 0;
  const game = connectShardGame<{ dx: number }>("zone", {
    subscriberId: "p1",
    baseUrl: "h",
    tickRate: 20,
    interpolationDelayMs: 100,
    now: () => time,
  });
  const me = game.predict<number>((x, input) => x + input.dx);
  let predicted = 0;
  game.onReplication((table, _summary, _tick, ack) => {
    const mine = table.get(1);
    if (mine) predicted = me.reconcile(mine.x, ack);
  });

  expect(game.frame(0)).toBe(-1);
  expect(game.send({ dx: 1 })).toBe(0); // not open yet

  const ws = sockets[0];
  expect(ws.url).toContain("v=2");
  ws.open();

  // Tick t is built at t * 50 ms and arrives 20 ms later. Entity 2 walks
  // 1 unit per tick.
  const deliver = (tick: number, ack: number, frame: Parameters<typeof replicationFrame>[2]) => {
    time = tick * 50 + 20;
    ws.deliver(replicationFrame(tick, ack, frame));
  };
  deliver(1, 0, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }, { id: 2, pos: [1, 0, 0] }] });
  for (let tick = 2; tick <= 10; tick++) {
    deliver(tick, 0, { update: [{ id: 2, from: [tick - 1, 0, 0], pos: [tick, 0, 0] }] });
  }

  // 100 ms (2 ticks) behind the newest tick, halfway between frames.
  const renderTick = game.frame(10 * 50 + 20 + 25);
  expect(renderTick).toBeCloseTo(8.5, 1);
  expect(game.entities.get(2)?.x).toBeCloseTo(8.5, 1);
  expect(game.entities.get(1)?.x).toBeCloseTo(0);
  expect(game.latest.get(2)?.x).toBeCloseTo(10);

  // Three inputs; the shard applies the first two, clamping the second.
  const s1 = game.send({ dx: 1 });
  const s2 = game.send({ dx: 1 });
  const s3 = game.send({ dx: 1 });
  expect([s1, s2, s3]).toEqual([1, 2, 3]);
  expect(JSON.parse(ws.sent[0])).toEqual({ input: { dx: 1 }, client_seq: 1 });
  deliver(11, 2, { update: [{ id: 1, from: [0, 0, 0], pos: [1.5, 0, 0] }] });
  expect(predicted).toBeCloseTo(2.5);
  expect(game.ack).toBe(2);
  // Sent at 520 ms, acknowledged by the frame at 570 ms.
  expect(game.rttMs).toBeCloseTo(50);

  // The shard refuses input 3: it drops out of the prediction.
  ws.deliver(rejection(11, 3));
  deliver(12, 2, {});
  expect(predicted).toBeCloseTo(1.5);

  // A reconnect drops inputs sent on the old connection.
  game.send({ dx: 5 });
  ws.close();
  await new Promise((r) => realSetTimeout(r, 0));
  const ws2 = sockets[1];
  ws2.open();
  time = 13 * 50 + 20;
  ws2.deliver(replicationFrame(13, 0, { full: true, spawn: [{ id: 1, pos: [1.5, 0, 0] }] }));
  expect(predicted).toBeCloseTo(1.5);
  expect(me.size).toBe(0);

  // Entity 2 was not in the full frame: it leaves at tick 13.
  game.frame(13 * 50 + 20 + 100);
  expect(game.entities.has(2)).toBe(false);
  expect(game.left).toEqual([2]);
  game.close();
});
