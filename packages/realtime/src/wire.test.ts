import { describe, expect, test } from "bun:test";
import { encode as msgpackEncode, decode as msgpackDecode } from "@msgpack/msgpack";

import {
  MAX_ACKS_PER_MESSAGE,
  SHARD_HEADER_LEN,
  ShardCodec,
  ShardFrameKind,
  decodeShardPayload,
  decodeShardRejection,
  encodeDatagramAckBatches,
  encodeShardInput,
  parseShardFrame,
} from "./wire";

function frame(kind: number, codec: number, tick: number, ack: number, payload: Uint8Array) {
  const buf = new ArrayBuffer(SHARD_HEADER_LEN + payload.length);
  const view = new DataView(buf);
  view.setUint8(0, kind);
  view.setUint8(1, codec);
  view.setUint32(2, Math.floor(tick / 0x1_0000_0000));
  view.setUint32(6, tick >>> 0);
  view.setUint32(10, Math.floor(ack / 0x1_0000_0000));
  view.setUint32(14, ack >>> 0);
  new Uint8Array(buf, SHARD_HEADER_LEN).set(payload);
  return buf;
}

describe("parseShardFrame", () => {
  test("reads kind, codec, tick, ack, and payload", () => {
    const f = parseShardFrame(
      frame(ShardFrameKind.Snapshot, ShardCodec.Json, 2 ** 33 + 5, 7, new TextEncoder().encode("42")),
    );
    expect(f.kind).toBe(1);
    expect(f.codec).toBe(0);
    expect(f.tick).toBe(2 ** 33 + 5);
    expect(f.ack).toBe(7);
    expect(decodeShardPayload(f.codec, f.payload)).toBe(42);
  });

  test("rejects a frame shorter than the header", () => {
    expect(() => parseShardFrame(new ArrayBuffer(10))).toThrow("too short");
  });
});

test("MessagePack payloads decode to objects", () => {
  const payload = msgpackEncode({ players: [{ id: "a", x: 1.5 }], tick: 3 });
  const f = parseShardFrame(frame(ShardFrameKind.Snapshot, ShardCodec.MessagePack, 3, 0, payload));
  expect(decodeShardPayload(f.codec, f.payload)).toEqual({ players: [{ id: "a", x: 1.5 }], tick: 3 });
});

test("rejection frames decode with camelCase fields", () => {
  const payload = msgpackEncode({ client_seq: 9, code: "rate_limited", message: "slow down" });
  const f = parseShardFrame(frame(ShardFrameKind.InputRejected, ShardCodec.MessagePack, 1, 8, payload));
  expect(decodeShardRejection(f.codec, f.payload)).toEqual({
    clientSeq: 9,
    code: "rate_limited",
    message: "slow down",
  });
});

test("an unknown codec needs a custom decoder", () => {
  const bytes = new Uint8Array([1, 2, 3]);
  expect(() => decodeShardPayload(ShardCodec.Custom, bytes)).toThrow("custom decoder");
  expect(decodeShardPayload(ShardCodec.Custom, bytes, (p) => p.length)).toBe(3);
});

test("inputs are MessagePack for MessagePack shards and JSON otherwise", () => {
  const bin = encodeShardInput(ShardCodec.MessagePack, { move: "n" }, 4);
  expect(bin).toBeInstanceOf(Uint8Array);
  expect(msgpackDecode(bin as Uint8Array)).toEqual({ input: { move: "n" }, client_seq: 4 });
  expect(encodeShardInput(null, 5, 1)).toBe('{"input":5,"client_seq":1}');
  expect(encodeShardInput(ShardCodec.Json, 5, 2)).toBe('{"input":5,"client_seq":2}');
});

test("ack batches stay within the datagram size", () => {
  const acks = Array.from({ length: 1300 }, (_, i) => [2_000_000 + i, 1_000_000] as [number, number]);
  const batches = encodeDatagramAckBatches(acks, 200);
  let total = 0;
  for (const b of batches) {
    expect(b.length).toBeLessThanOrEqual(200);
    expect(b[0]).toBe(1);
    // The count is the second byte (under 128 here).
    total += b[1];
  }
  expect(total).toBe(1300);
  const big = encodeDatagramAckBatches(Array.from({ length: 1300 }, (_, i) => [i % 100, 1] as [number, number]), 1_000_000);
  expect(big.length).toBe(Math.ceil(1300 / MAX_ACKS_PER_MESSAGE));
});
