/**
 * The shard wire protocol, version 2 (see `pylon_realtime::wire` in Rust).
 *
 * A client asks for it with `?v=2` on the shard WebSocket URL. Each server
 * message is a binary frame with an 18-byte header:
 *
 *   0   1  frame kind: 1 snapshot, 2 input rejected, 3 entity replication,
 *            4 transfer (JSON `{ shard, ticket }`: the last frame; reconnect there),
 *            6 closing (WebTransport only, JSON `{ code, reason }`)
 *   1   1  codec: 0 JSON, 1 MessagePack, 2 bincode, 3 custom, 4 replication
 *   2   8  tick (u64 big-endian)
 *   10  8  ack: highest client_seq the shard processed for this subscriber (0 = none)
 *   18  .. payload in the codec
 *
 * Inputs go up as `{ input, client_seq }`: JSON in a text frame, or the
 * shard's codec in a binary frame.
 *
 * Over WebTransport (wire version 3 in Rust) the same frames travel on one
 * bidirectional stream, each after a 4-byte big-endian length, and entity
 * updates travel as QUIC datagrams (`EntityTable.applyDatagram`). The
 * client's hello, its inputs, and its datagram acks are described at
 * `encodeWebTransportHello`, `ShardClientMessage`, and `encodeDatagramAcks`.
 */

import { decode as msgpackDecode, encode as msgpackEncode } from "@msgpack/msgpack";

export const SHARD_PROTOCOL_VERSION = 2;
export const SHARD_HEADER_LEN = 18;

export const ShardFrameKind = {
  Snapshot: 1,
  InputRejected: 2,
  /** An entity replication frame: apply it to an `EntityTable`. */
  Replication: 3,
  /**
   * The subscriber moved to another shard (`ShardTransferNotice`, JSON). The
   * last frame on the connection.
   */
  Transfer: 4,
  /** A datagram frame (wire version 3 WebSocket only). */
  Datagram: 5,
  /**
   * WebTransport only: the server is about to close the session, with
   * JSON `{ code, reason }` (`WEBTRANSPORT_CLOSE`). The client closes it.
   */
  Closing: 6,
} as const;

/** Where the subscriber went: connect to `shard` with `ticket`. */
export interface ShardTransferNotice {
  shard: string;
  ticket: string;
}

export const ShardCodec = {
  Json: 0,
  MessagePack: 1,
  Bincode: 2,
  Custom: 3,
  /** The replication frame format (frame kind 3). */
  Replication: 4,
} as const;

export interface ShardFrame {
  kind: number;
  codec: number;
  tick: number;
  ack: number;
  payload: Uint8Array;
}

/** Why an input did not take effect. */
export interface ShardInputRejection {
  clientSeq: number | null;
  /**
   * `unauthorized`, `rate_limited`, `queue_full`, `invalid`, `stopped`,
   * `transferring`, or `apply_failed`.
   */
  code: string;
  message: string;
}

/**
 * Decode a payload in a codec the JS client does not know (bincode, or a
 * game's own codec `3`).
 */
export type ShardPayloadDecoder = (payload: Uint8Array, codec: number) => unknown;

function readU64(view: DataView, offset: number): number {
  return view.getUint32(offset) * 0x1_0000_0000 + view.getUint32(offset + 4);
}

/** Split a version 2 frame into its header fields and payload. */
export function parseShardFrame(data: ArrayBuffer): ShardFrame {
  if (data.byteLength < SHARD_HEADER_LEN) {
    throw new Error(`shard frame too short (${data.byteLength} bytes)`);
  }
  const view = new DataView(data);
  return {
    kind: view.getUint8(0),
    codec: view.getUint8(1),
    tick: readU64(view, 2),
    ack: readU64(view, 10),
    payload: new Uint8Array(data, SHARD_HEADER_LEN),
  };
}

/** Decode a payload. JSON and MessagePack are built in. */
export function decodeShardPayload(
  codec: number,
  payload: Uint8Array,
  custom?: ShardPayloadDecoder,
): unknown {
  switch (codec) {
    case ShardCodec.Json:
      return JSON.parse(new TextDecoder().decode(payload));
    case ShardCodec.MessagePack:
      return msgpackDecode(payload);
    default:
      if (custom) return custom(payload, codec);
      throw new Error(
        `shard codec ${codec} needs a custom decoder (pass \`decode\` to connectShard)`,
      );
  }
}

/** The payload of an input-rejected frame, with camelCase fields. */
export function decodeShardRejection(
  codec: number,
  payload: Uint8Array,
  custom?: ShardPayloadDecoder,
): ShardInputRejection {
  const raw = decodeShardPayload(codec, payload, custom) as {
    client_seq?: number | null;
    code?: string;
    message?: string;
  };
  return {
    clientSeq: typeof raw.client_seq === "number" ? raw.client_seq : null,
    code: String(raw.code ?? "invalid"),
    message: String(raw.message ?? ""),
  };
}

/**
 * Encode an input envelope. MessagePack shards get a binary frame; every
 * other codec (and a connection that has not seen a frame yet) gets JSON
 * text, which the server always accepts.
 */
export function encodeShardInput(
  codec: number | null,
  input: unknown,
  clientSeq: number,
  viewTick?: number,
): string | Uint8Array {
  const envelope: { input: unknown; client_seq: number; view_tick?: number } = {
    input,
    client_seq: clientSeq,
  };
  // The tick the client was drawing, for a shard with lag compensation.
  if (viewTick !== undefined && Number.isFinite(viewTick)) envelope.view_tick = viewTick;
  if (codec === ShardCodec.MessagePack) return msgpackEncode(envelope);
  return JSON.stringify(envelope);
}

/** Acks one message may carry (the server refuses more). */
export const MAX_ACKS_PER_MESSAGE = 512;

/**
 * Type bytes of the client's messages on a WebTransport stream (the first
 * byte; the rest is the message).
 */
export const ShardClientMessage = {
  /** An input envelope in the shard's codec (MessagePack shards). */
  Input: 0,
  /** Datagram acks (`encodeDatagramAcks`). */
  Acks: 1,
  /** An input envelope as JSON. */
  JsonInput: 2,
} as const;

/** WebTransport session close codes the server uses. */
export const WEBTRANSPORT_CLOSE = {
  Normal: 0,
  /** Refused: bad credentials, an unknown shard. */
  Policy: 1,
  /** The client broke the protocol. */
  Protocol: 2,
  /** Try again: the client was too slow, or the server was busy. */
  Again: 3,
} as const;

/** What `GET /_pylon/shard/webtransport` returns. */
export interface WebTransportInfo {
  /** The `https://` URL of the WebTransport endpoint. */
  url: string;
  /**
   * SHA-256 hashes of the server's self-signed certificates, for
   * `serverCertificateHashes`. Empty when the certificate has a public CA.
   */
  certHashes: Uint8Array[];
}

/** Parse the body of `GET /_pylon/shard/webtransport`. */
export function decodeWebTransportInfo(body: unknown): WebTransportInfo {
  const raw = body as { url?: unknown; certHashes?: unknown };
  if (typeof raw?.url !== "string") throw new Error("WebTransport info without a url");
  const hashes = Array.isArray(raw.certHashes) ? raw.certHashes : [];
  return {
    url: raw.url,
    certHashes: hashes.map((h) => {
      if (typeof h !== "string") throw new Error("a WebTransport certificate hash is not a string");
      const bin = atob(h);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      if (bytes.length !== 32) throw new Error(`a ${bytes.length}-byte certificate hash`);
      return bytes;
    }),
  };
}

function pushVarint(out: number[], v: number): void {
  if (!Number.isSafeInteger(v) || v < 0) throw new Error(`varint out of range: ${v}`);
  while (v >= 0x80) {
    out.push((v % 0x80) | 0x80);
    v = Math.floor(v / 0x80);
  }
  out.push(v);
}

/**
 * Datagram acks: the type byte `ShardClientMessage.Acks`, a varint count,
 * then per ack the datagram's frame number and `EntityTable.streamTick`
 * when the client applied it, both varints. Sent as a datagram, or on the
 * stream.
 */
export function encodeDatagramAcks(acks: ReadonlyArray<readonly [number, number]>): Uint8Array {
  const out: number[] = [ShardClientMessage.Acks];
  pushVarint(out, acks.length);
  for (const [frame, applied] of acks) {
    pushVarint(out, frame);
    pushVarint(out, applied);
  }
  return Uint8Array.from(out);
}

/** Bytes `pushVarint` writes for `v`. */
function varintLen(v: number): number {
  let n = 1;
  while (v >= 0x80) {
    v = Math.floor(v / 0x80);
    n += 1;
  }
  return n;
}

/**
 * `encodeDatagramAcks` split into messages of at most `maxBytes` bytes and
 * `MAX_ACKS_PER_MESSAGE` acks each, so every one fits in a datagram.
 */
export function encodeDatagramAckBatches(
  acks: ReadonlyArray<readonly [number, number]>,
  maxBytes: number,
): Uint8Array[] {
  const out: Uint8Array[] = [];
  let start = 0;
  // Type byte plus the count, which is below 2^14 (two varint bytes).
  let size = 3;
  for (let i = 0; i < acks.length; i++) {
    const [frame, applied] = acks[i];
    const n = varintLen(frame) + varintLen(applied);
    if (i > start && (size + n > maxBytes || i - start === MAX_ACKS_PER_MESSAGE)) {
      out.push(encodeDatagramAcks(acks.slice(start, i)));
      start = i;
      size = 3;
    }
    size += n;
  }
  if (start < acks.length) out.push(encodeDatagramAcks(acks.slice(start)));
  return out;
}

/** A message for a WebTransport stream: a 4-byte big-endian length, then `bytes`. */
export function lengthPrefixed(bytes: Uint8Array): Uint8Array {
  const out = new Uint8Array(4 + bytes.length);
  new DataView(out.buffer).setUint32(0, bytes.length);
  out.set(bytes, 4);
  return out;
}

/** The first message on a WebTransport stream: who connects to which shard. */
export function encodeWebTransportHello(hello: {
  shard: string;
  sid: string;
  ticket?: string;
  token?: string;
}): Uint8Array {
  return lengthPrefixed(new TextEncoder().encode(JSON.stringify(hello)));
}

/**
 * An input as a WebTransport stream message: `encodeShardInput`'s output
 * after its type byte.
 */
export function encodeWebTransportInput(encoded: string | Uint8Array): Uint8Array {
  const body = typeof encoded === "string" ? new TextEncoder().encode(encoded) : encoded;
  const msg = new Uint8Array(1 + body.length);
  msg[0] = typeof encoded === "string" ? ShardClientMessage.JsonInput : ShardClientMessage.Input;
  msg.set(body, 1);
  return lengthPrefixed(msg);
}

/** A frame on a WebTransport stream larger than this ends the session. */
const MAX_STREAM_FRAME = 64 * 1024 * 1024;

/**
 * Splits a WebTransport stream's bytes into its length-prefixed frames.
 * Chunks can end anywhere: in a length, in a frame.
 */
export class StreamFrames {
  private chunks: Uint8Array[] = [];
  private buffered = 0;

  /** Add a chunk and return the frames it completed. */
  push(chunk: Uint8Array): ArrayBuffer[] {
    if (chunk.length === 0) return [];
    this.chunks.push(chunk);
    this.buffered += chunk.length;
    const frames: ArrayBuffer[] = [];
    while (this.buffered >= 4) {
      const head = this.peek(4);
      const len = new DataView(head.buffer, head.byteOffset, 4).getUint32(0);
      if (len > MAX_STREAM_FRAME) throw new Error(`a ${len}-byte stream frame`);
      if (this.buffered < 4 + len) break;
      this.consume(4);
      const frame = new Uint8Array(len);
      this.consume(len, frame);
      frames.push(frame.buffer);
    }
    return frames;
  }

  private peek(n: number): Uint8Array {
    if (this.chunks[0].length >= n) return this.chunks[0].subarray(0, n);
    const out = new Uint8Array(n);
    let at = 0;
    for (const c of this.chunks) {
      const part = c.subarray(0, n - at);
      out.set(part, at);
      at += part.length;
      if (at === n) break;
    }
    return out;
  }

  private consume(n: number, output?: Uint8Array): void {
    let left = n;
    while (left > 0) {
      const c = this.chunks[0];
      if (output) output.set(c.subarray(0, Math.min(c.length, left)), n - left);
      if (c.length <= left) {
        this.chunks.shift();
        left -= c.length;
      } else {
        this.chunks[0] = c.subarray(left);
        left = 0;
      }
    }
    this.buffered -= n;
  }
}
