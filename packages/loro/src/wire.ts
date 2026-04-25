// ---------------------------------------------------------------------------
// CRDT WebSocket wire format
//
// Mirror of `crates/router/src/lib.rs::encode_crdt_frame`. Frame layout:
//
//   [type: u8] [entity_len: u16 BE] [entity utf8]
//   [row_id_len: u16 BE] [row_id utf8] [payload bytes]
//
// Type bytes:
//   0x10 = full Loro snapshot
//   0x11 = incremental Loro update
//
// Keep this in sync with the Rust encoder. A change to either side
// without a matching change to the other corrupts every CRDT message
// in flight.
// ---------------------------------------------------------------------------

export const CRDT_FRAME_SNAPSHOT = 0x10;
export const CRDT_FRAME_UPDATE = 0x11;

export interface CrdtFrame {
  /** 0x10 (snapshot) or 0x11 (incremental update). */
  type: number;
  /** Entity name as declared in the manifest. */
  entity: string;
  /** Row ID — 40-char hex for Pylon-generated rows. */
  rowId: string;
  /** Loro binary payload. Snapshot or update bytes depending on `type`. */
  payload: Uint8Array;
}

/**
 * Decode a binary CRDT frame received over the WebSocket. Returns
 * `null` on any parse failure (truncated frame, length-header overrun)
 * — caller logs and drops; the next valid frame is independent.
 *
 * The router encoder bails on entity / row_id strings >65 KiB, so the
 * decoder's malformed-input path is genuinely unreachable for frames
 * the server emits. The defensive checks exist for cases where the
 * client receives bytes from a custom proxy / test fixture / future
 * protocol extension.
 */
export function decodeCrdtFrame(bytes: Uint8Array): CrdtFrame | null {
  // Header is at minimum: type(1) + entity_len(2) + row_id_len(2) = 5 bytes.
  if (bytes.length < 5) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const type = view.getUint8(0);
  const entityLen = view.getUint16(1, false /* big-endian */);
  const entityStart = 3;
  const entityEnd = entityStart + entityLen;
  if (entityEnd + 2 > bytes.length) return null;

  const rowIdLen = view.getUint16(entityEnd, false);
  const rowIdStart = entityEnd + 2;
  const rowIdEnd = rowIdStart + rowIdLen;
  if (rowIdEnd > bytes.length) return null;

  const decoder = new TextDecoder();
  const entity = decoder.decode(bytes.subarray(entityStart, entityEnd));
  const rowId = decoder.decode(bytes.subarray(rowIdStart, rowIdEnd));
  const payload = bytes.subarray(rowIdEnd);

  return { type, entity, rowId, payload };
}

/**
 * Encode a frame in the same format. Useful for tests / for any
 * eventual client-to-server CRDT push (not yet wired). Throws on
 * length-header overrun rather than truncating, matching the Rust
 * encoder's failure mode.
 */
export function encodeCrdtFrame(
  type: number,
  entity: string,
  rowId: string,
  payload: Uint8Array,
): Uint8Array {
  const encoder = new TextEncoder();
  const entityBytes = encoder.encode(entity);
  const rowIdBytes = encoder.encode(rowId);
  if (entityBytes.length > 0xffff) {
    throw new Error(
      `CRDT frame: entity name ${entityBytes.length} bytes exceeds u16 length limit (65535)`,
    );
  }
  if (rowIdBytes.length > 0xffff) {
    throw new Error(
      `CRDT frame: row_id ${rowIdBytes.length} bytes exceeds u16 length limit (65535)`,
    );
  }
  const out = new Uint8Array(
    1 + 2 + entityBytes.length + 2 + rowIdBytes.length + payload.length,
  );
  const view = new DataView(out.buffer);
  let offset = 0;
  view.setUint8(offset, type);
  offset += 1;
  view.setUint16(offset, entityBytes.length, false);
  offset += 2;
  out.set(entityBytes, offset);
  offset += entityBytes.length;
  view.setUint16(offset, rowIdBytes.length, false);
  offset += 2;
  out.set(rowIdBytes, offset);
  offset += rowIdBytes.length;
  out.set(payload, offset);
  return out;
}
