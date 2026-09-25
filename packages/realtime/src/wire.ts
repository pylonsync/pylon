/**
 * The shard wire protocol, version 2 (see `pylon_realtime::wire` in Rust).
 *
 * A client asks for it with `?v=2` on the shard WebSocket URL. Each server
 * message is a binary frame with an 18-byte header:
 *
 *   0   1  frame kind: 1 snapshot, 2 input rejected, 3 entity replication,
 *            4 transfer (JSON `{ shard, ticket }`: the last frame; reconnect there)
 *   1   1  codec: 0 JSON, 1 MessagePack, 2 bincode, 3 custom, 4 replication
 *   2   8  tick (u64 big-endian)
 *   10  8  ack: highest client_seq the shard processed for this subscriber (0 = none)
 *   18  .. payload in the codec
 *
 * Inputs go up as `{ input, client_seq }`: JSON in a text frame, or the
 * shard's codec in a binary frame.
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
): string | Uint8Array {
  const envelope = { input, client_seq: clientSeq };
  if (codec === ShardCodec.MessagePack) return msgpackEncode(envelope);
  return JSON.stringify(envelope);
}
