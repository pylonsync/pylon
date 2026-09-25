/**
 * A replication frame encoder for tests (the real one is in Rust:
 * `pylon_replication::frame`). Positions are world units at a precision of
 * 0.01; the encoder quantizes them.
 */

import { SHARD_HEADER_LEN, ShardCodec, ShardFrameKind } from "../src/wire";

const PRECISION = 0.01;

export interface TestEntity {
  id: number;
  pos?: [number, number, number];
  /** Component id to bytes; null removes it (updates only). */
  components?: Record<number, number[] | null>;
}

export interface TestFrame {
  full?: boolean;
  despawn?: number[];
  spawn?: TestEntity[];
  /** `pos` here is the new position; the encoder needs the old one. */
  update?: Array<TestEntity & { from?: [number, number, number] }>;
}

function varint(out: number[], v: bigint): void {
  do {
    let b = Number(v & 0x7fn);
    v >>= 7n;
    if (v > 0n) b |= 0x80;
    out.push(b);
  } while (v > 0n);
}

function zigzag(out: number[], v: number): void {
  const b = BigInt(v);
  varint(out, b >= 0n ? b << 1n : ((-b) << 1n) - 1n);
}

function q(v: number): number {
  return Math.round(v / PRECISION);
}

function components(out: number[], c: Record<number, number[] | null> = {}): void {
  const keys = Object.keys(c).map(Number);
  varint(out, BigInt(keys.length));
  for (const k of keys) {
    out.push(k);
    const v = c[k];
    if (v === null) {
      varint(out, 0n);
    } else {
      varint(out, BigInt(v.length + 1));
      out.push(...v);
    }
  }
}

function ids<T extends { id: number }>(out: number[], list: T[], each: (e: T) => void): void {
  varint(out, BigInt(list.length));
  let last: number | null = null;
  for (const e of [...list].sort((a, b) => a.id - b.id)) {
    varint(out, BigInt(last === null ? e.id : e.id - last));
    last = e.id;
    each(e);
  }
}

/** A replication payload (what `EntityTable.apply` takes). */
export function replicationPayload(f: TestFrame): Uint8Array {
  const out: number[] = [1, f.full ? 1 : 0];
  const p = new DataView(new ArrayBuffer(4));
  p.setFloat32(0, PRECISION, true);
  out.push(...new Uint8Array(p.buffer));
  ids(out, (f.despawn ?? []).map((id) => ({ id })), () => {});
  ids(out, f.spawn ?? [], (e) => {
    const [x, y, z] = e.pos ?? [0, 0, 0];
    zigzag(out, q(x));
    zigzag(out, q(y));
    zigzag(out, q(z));
    components(out, e.components);
  });
  ids(out, f.update ?? [], (e) => {
    const from = e.from ?? [0, 0, 0];
    const to = e.pos ?? from;
    const d = [0, 1, 2].map((i) => q(to[i]) - q(from[i]));
    let mask = 0;
    d.forEach((v, i) => {
      if (v !== 0) mask |= 1 << i;
    });
    if (e.components) mask |= 8;
    out.push(mask);
    d.forEach((v) => {
      if (v !== 0) zigzag(out, v);
    });
    if (e.components) components(out, e.components);
  });
  return new Uint8Array(out);
}

/** A whole shard frame (header and payload) as the WebSocket delivers it. */
export function shardFrame(
  kind: number,
  codec: number,
  tick: number,
  ack: number,
  payload: Uint8Array,
): ArrayBuffer {
  const buf = new ArrayBuffer(SHARD_HEADER_LEN + payload.length);
  const view = new DataView(buf);
  view.setUint8(0, kind);
  view.setUint8(1, codec);
  view.setBigUint64(2, BigInt(tick));
  view.setBigUint64(10, BigInt(ack));
  new Uint8Array(buf, SHARD_HEADER_LEN).set(payload);
  return buf;
}

export function replicationFrame(tick: number, ack: number, f: TestFrame): ArrayBuffer {
  return shardFrame(ShardFrameKind.Replication, ShardCodec.Replication, tick, ack, replicationPayload(f));
}
