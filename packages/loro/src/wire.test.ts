// ---------------------------------------------------------------------------
// Wire-format tests. Mirror the Rust-side tests in
// `crates/router/src/lib.rs::crdt_frame_tests` so any divergence
// between encoder and decoder fails loud here before it ships.
// Run via: `bun test packages/loro/src/wire.test.ts`
// ---------------------------------------------------------------------------

import { test, expect } from "bun:test";
import {
  CRDT_FRAME_SNAPSHOT,
  CRDT_FRAME_UPDATE,
  decodeCrdtFrame,
  encodeCrdtFrame,
} from "./wire";

test("encode + decode round-trips a snapshot frame", () => {
  const payload = new Uint8Array([0xab, 0xcd, 0xef]);
  const frame = encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "Message", "msg_123", payload);
  // Header: 1 + 2 + 7 ("Message") + 2 + 7 ("msg_123") + 3 = 22 bytes.
  expect(frame.length).toBe(22);

  const decoded = decodeCrdtFrame(frame);
  expect(decoded).not.toBeNull();
  expect(decoded!.type).toBe(CRDT_FRAME_SNAPSHOT);
  expect(decoded!.entity).toBe("Message");
  expect(decoded!.rowId).toBe("msg_123");
  expect(Array.from(decoded!.payload)).toEqual([0xab, 0xcd, 0xef]);
});

test("matches Rust encoder byte-for-byte for the same input", () => {
  // Identical to the `roundtrip_header_layout` test in
  // crates/router/src/lib.rs::crdt_frame_tests. If this fails the
  // wire formats have drifted between Rust and TS.
  const frame = encodeCrdtFrame(
    CRDT_FRAME_SNAPSHOT,
    "Message",
    "msg_123",
    new Uint8Array([0xab, 0xcd, 0xef]),
  );
  expect(frame[0]).toBe(0x10);
  expect(Array.from(frame.subarray(1, 3))).toEqual([0, 7]);
  expect(new TextDecoder().decode(frame.subarray(3, 10))).toBe("Message");
  expect(Array.from(frame.subarray(10, 12))).toEqual([0, 7]);
  expect(new TextDecoder().decode(frame.subarray(12, 19))).toBe("msg_123");
  expect(Array.from(frame.subarray(19, 22))).toEqual([0xab, 0xcd, 0xef]);
});

test("empty payload still carries headers", () => {
  const frame = encodeCrdtFrame(CRDT_FRAME_UPDATE, "X", "y", new Uint8Array(0));
  expect(frame.length).toBe(7);
  expect(frame[0]).toBe(0x11);

  const decoded = decodeCrdtFrame(frame);
  expect(decoded!.entity).toBe("X");
  expect(decoded!.rowId).toBe("y");
  expect(decoded!.payload.length).toBe(0);
});

test("entity > u16 max throws", () => {
  const huge = "x".repeat(0x10000);
  expect(() =>
    encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, huge, "y", new Uint8Array(0)),
  ).toThrow(/exceeds u16/);
});

test("row_id > u16 max throws", () => {
  const huge = "x".repeat(0x10000);
  expect(() =>
    encodeCrdtFrame(CRDT_FRAME_SNAPSHOT, "X", huge, new Uint8Array(0)),
  ).toThrow(/exceeds u16/);
});

test("decodes a truncated frame as null instead of throwing", () => {
  // A frame that claims entity_len=10 but only carries 3 bytes of entity.
  const truncated = new Uint8Array([
    CRDT_FRAME_SNAPSHOT,
    0x00,
    0x0a, // entity_len = 10
    0x41,
    0x42,
    0x43, // only "ABC" — 3 bytes, not 10
  ]);
  expect(decodeCrdtFrame(truncated)).toBeNull();
});

test("decodes a too-short header (<5 bytes) as null", () => {
  expect(decodeCrdtFrame(new Uint8Array([0x10, 0x00]))).toBeNull();
  expect(decodeCrdtFrame(new Uint8Array(0))).toBeNull();
});

test("UTF-8 multi-byte chars in entity / row_id round-trip", () => {
  // 文書 = 6 bytes UTF-8, not 2 chars × 1 byte. Entity-name length
  // should be the BYTE length the encoder writes — exercising the
  // u16-as-bytes-not-chars contract on both sides.
  const frame = encodeCrdtFrame(
    CRDT_FRAME_SNAPSHOT,
    "文書",
    "行_42",
    new Uint8Array(0),
  );
  const decoded = decodeCrdtFrame(frame);
  expect(decoded!.entity).toBe("文書");
  expect(decoded!.rowId).toBe("行_42");
});
