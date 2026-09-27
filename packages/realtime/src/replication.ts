/**
 * Entity replication frames (see `pylon_replication::frame` in Rust).
 *
 * A shard that replicates entities sends each subscriber, per tick, the
 * entities that left its view (despawn), came into view (spawn, full
 * state), or changed (update: only the axes that moved, as a quantized
 * difference, and the components that changed or were removed).
 * {@link EntityTable} applies them; after each frame it holds what the
 * server says this subscriber sees.
 *
 * ```text
 * u8      version (1)
 * u8      flags: bit 0 FULL: clear the table first
 * f32 LE  precision
 * varint  despawn count, ids (first absolute, then differences; ascending)
 * varint  spawn count, per entity: id, x, y, z (zigzag, quantized), components
 * varint  update count, per entity: id, u8 mask (1 x, 2 y, 4 z, 8 components),
 *         changed axes (zigzag differences), components if bit 8
 * components: varint count, per component: u8 id, varint (length + 1),
 *         bytes; length field 0 means removed
 * ```
 *
 * JavaScript numbers hold integers exactly up to 2^53. Entity ids and
 * quantized positions must stay below that; a frame that exceeds it is
 * refused rather than rounded.
 *
 * **Datagrams** (`pylon_replication::datagram`): on WebTransport, spawns,
 * despawns, and full frames come as frames on the stream, and updates as
 * datagrams that may be lost, duplicated, or reordered. A datagram update
 * carries the entity's absolute position and the generation (spawn count
 * of its id) it is for. {@link EntityTable.applyDatagram} applies an update
 * only to the entity's current generation and only when it is newer than
 * the entity's last datagram, and skips it otherwise.
 *
 * ```text
 * u8      version (2)
 * varint  frame number, tick, input ack
 * f32 LE  precision
 * varint  update count, per entity (ids ascending, delta-coded): id,
 *         u8 generation, u8 mask, the masked axes (zigzag, absolute),
 *         components if bit 8
 * ```
 */

export const REPLICATION_VERSION = 1;
export const DATAGRAM_VERSION = 2;

const FLAG_FULL = 1;
const MASK_X = 1;
const MASK_Y = 2;
const MASK_Z = 4;
const MASK_COMPONENTS = 8;

/** Longest id list a frame may declare. */
const MAX_COUNT = 1 << 24;

export class ReplicationError extends Error {
  constructor(message: string) {
    super(`replication frame: ${message}`);
    this.name = "ReplicationError";
  }
}

/** One entity as the client sees it. */
export interface ReplicatedEntity {
  readonly id: number;
  /** Quantized position; `x/y/z` are these times the frame precision. */
  qx: number;
  qy: number;
  qz: number;
  x: number;
  y: number;
  z: number;
  /** Component id to bytes, in the game's own encoding. */
  components: Map<number, Uint8Array>;
}

/** What one datagram did. */
export interface DatagramSummary {
  frame: number;
  tick: number;
  ack: number;
  /** Entities it updated. */
  updated: number[];
  /**
   * Updates it skipped: an unknown entity, another generation, an older
   * datagram than the entity's last, or another precision.
   */
  skipped: number;
}

/** What one frame did. */
export interface ReplicationSummary {
  full: boolean;
  spawned: number[];
  updated: number[];
  despawned: number[];
}

/**
 * A datagram's frame number, tick, and input ack, without applying it.
 * Throws on bytes that are not a datagram.
 */
export function readDatagramHeader(datagram: Uint8Array): { frame: number; tick: number; ack: number } {
  const r = new Reader(datagram);
  const version = r.u8();
  if (version !== DATAGRAM_VERSION) throw new ReplicationError(`datagram version ${version}`);
  return {
    frame: toSafe(r.varint(), "frame number"),
    tick: toSafe(r.varint(), "tick"),
    ack: toSafe(r.varint(), "ack"),
  };
}

class Reader {
  private offset = 0;
  private readonly view: DataView;

  constructor(private readonly bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  get done(): boolean {
    return this.offset >= this.bytes.length;
  }

  u8(): number {
    if (this.offset >= this.bytes.length) throw new ReplicationError("ends early");
    return this.bytes[this.offset++];
  }

  f32(): number {
    if (this.offset + 4 > this.bytes.length) throw new ReplicationError("ends early");
    const v = this.view.getFloat32(this.offset, true);
    this.offset += 4;
    return v;
  }

  /** An unsigned LEB128 varint, as a bigint (exact to 64 bits). */
  varint(): bigint {
    let v = 0n;
    for (let i = 0; i < 10; i++) {
      const b = this.u8();
      const part = BigInt(b & 0x7f);
      if (i === 9 && part > 1n) throw new ReplicationError("varint past 64 bits");
      v |= part << BigInt(7 * i);
      if ((b & 0x80) === 0) return v;
    }
    throw new ReplicationError("varint too long");
  }

  /** A zigzag signed varint, as a bigint. */
  zigzag(): bigint {
    const z = this.varint();
    return (z >> 1n) ^ -(z & 1n);
  }

  take(n: number): Uint8Array {
    if (this.offset + n > this.bytes.length) throw new ReplicationError("component runs past the end");
    const out = this.bytes.slice(this.offset, this.offset + n);
    this.offset += n;
    return out;
  }
}

function toSafe(v: bigint, what: string): number {
  if (v > BigInt(Number.MAX_SAFE_INTEGER) || v < BigInt(Number.MIN_SAFE_INTEGER)) {
    throw new ReplicationError(`${what} ${v} is beyond 2^53`);
  }
  return Number(v);
}

function count(r: Reader): number {
  const n = r.varint();
  if (n > BigInt(MAX_COUNT)) throw new ReplicationError("count too large");
  return Number(n);
}

function nextId(r: Reader, last: number | null): number {
  const v = r.varint();
  if (last === null) return toSafe(v, "entity id");
  if (v === 0n) throw new ReplicationError("ids do not ascend");
  return toSafe(BigInt(last) + v, "entity id");
}

function readComponents(r: Reader, into: Map<number, Uint8Array>): void {
  const n = r.varint();
  if (n > 256n) throw new ReplicationError("more than 256 components");
  for (let i = 0; i < Number(n); i++) {
    const id = r.u8();
    const len = r.varint();
    if (len === 0n) {
      into.delete(id);
      continue;
    }
    if (len - 1n > BigInt(Number.MAX_SAFE_INTEGER)) throw new ReplicationError("component too large");
    into.set(id, r.take(Number(len - 1n)));
  }
}

/**
 * The entities one subscriber has been told about. Apply every
 * replication frame in order; a frame that fails to apply means the
 * connection is out of sync and should be reopened (the server then sends
 * a full frame).
 */
export class EntityTable {
  readonly entities = new Map<number, ReplicatedEntity>();
  /** World units per quantization step, from the last frame. */
  precision = 0.01;
  /** Spawns of each id so far, mod 256: the generation a datagram names. */
  readonly generations = new Map<number, number>();
  /** The last datagram applied to each entity. */
  readonly datagramFrames = new Map<number, number>();
  /**
   * Frames applied so far. A client acks each datagram with this count,
   * so the server knows which spawns it had.
   */
  framesApplied = 0;

  get size(): number {
    return this.entities.size;
  }

  get(id: number): ReplicatedEntity | undefined {
    return this.entities.get(id);
  }

  /**
   * Forget everything: a new connection starts over, and its server counts
   * generations and frames from zero.
   */
  clear(): void {
    this.entities.clear();
    this.generations.clear();
    this.datagramFrames.clear();
    this.framesApplied = 0;
  }

  apply(frame: Uint8Array): ReplicationSummary {
    const r = new Reader(frame);
    const version = r.u8();
    if (version !== REPLICATION_VERSION) throw new ReplicationError(`version ${version}`);
    const full = (r.u8() & FLAG_FULL) !== 0;
    const precision = r.f32();
    if (!Number.isFinite(precision) || precision <= 0) throw new ReplicationError("bad precision");
    if (full) {
      this.entities.clear();
      this.datagramFrames.clear();
    }
    this.precision = precision;
    const summary: ReplicationSummary = { full, spawned: [], updated: [], despawned: [] };

    let n = count(r);
    let last: number | null = null;
    for (let i = 0; i < n; i++) {
      last = nextId(r, last);
      this.entities.delete(last);
      this.datagramFrames.delete(last);
      summary.despawned.push(last);
    }

    n = count(r);
    last = null;
    for (let i = 0; i < n; i++) {
      last = nextId(r, last);
      const qx = toSafe(r.zigzag(), "position");
      const qy = toSafe(r.zigzag(), "position");
      const qz = toSafe(r.zigzag(), "position");
      const components = new Map<number, Uint8Array>();
      readComponents(r, components);
      this.entities.set(last, {
        id: last,
        qx,
        qy,
        qz,
        x: qx * precision,
        y: qy * precision,
        z: qz * precision,
        components,
      });
      this.datagramFrames.delete(last);
      this.generations.set(last, ((this.generations.get(last) ?? 0) + 1) & 0xff);
      summary.spawned.push(last);
    }

    n = count(r);
    last = null;
    for (let i = 0; i < n; i++) {
      last = nextId(r, last);
      const mask = r.u8();
      const e = this.entities.get(last);
      if (!e) throw new ReplicationError(`update for unknown entity ${last}`);
      if (mask & MASK_X) e.qx = toSafe(BigInt(e.qx) + r.zigzag(), "position");
      if (mask & MASK_Y) e.qy = toSafe(BigInt(e.qy) + r.zigzag(), "position");
      if (mask & MASK_Z) e.qz = toSafe(BigInt(e.qz) + r.zigzag(), "position");
      if (mask & MASK_COMPONENTS) readComponents(r, e.components);
      summary.updated.push(last);
    }
    if (!r.done) throw new ReplicationError("trailing bytes");

    // Positions follow the frame's precision (a full frame may change it).
    for (const e of this.entities.values()) {
      e.x = e.qx * precision;
      e.y = e.qy * precision;
      e.z = e.qz * precision;
    }
    this.framesApplied += 1;
    return summary;
  }

  /**
   * Apply one datagram. It changes nothing it cannot apply exactly (see
   * `DatagramSummary.skipped`). Throws only on bytes that are not a
   * datagram.
   */
  applyDatagram(datagram: Uint8Array): DatagramSummary {
    const r = new Reader(datagram);
    const version = r.u8();
    if (version !== DATAGRAM_VERSION) throw new ReplicationError(`datagram version ${version}`);
    const frame = toSafe(r.varint(), "frame number");
    const tick = toSafe(r.varint(), "tick");
    const ack = toSafe(r.varint(), "ack");
    const precision = r.f32();
    const n = r.varint();
    if (n > 1n << 16n) throw new ReplicationError("count too large");
    const summary: DatagramSummary = { frame, tick, ack, updated: [], skipped: 0 };
    // Positions in another precision mean nothing to this table: the full
    // frame of the new precision is still on its way.
    const usable = precision === Math.fround(this.precision);
    let last: number | null = null;
    for (let i = 0; i < Number(n); i++) {
      last = nextId(r, last);
      const generation = r.u8();
      const mask = r.u8();
      const qx = mask & MASK_X ? toSafe(r.zigzag(), "position") : null;
      const qy = mask & MASK_Y ? toSafe(r.zigzag(), "position") : null;
      const qz = mask & MASK_Z ? toSafe(r.zigzag(), "position") : null;
      const changes = new Map<number, Uint8Array>();
      const removed: number[] = [];
      if (mask & MASK_COMPONENTS) {
        const count = r.varint();
        if (count > 256n) throw new ReplicationError("more than 256 components");
        for (let c = 0; c < Number(count); c++) {
          const id = r.u8();
          const len = r.varint();
          if (len === 0n) {
            removed.push(id);
            continue;
          }
          if (len - 1n > BigInt(Number.MAX_SAFE_INTEGER)) throw new ReplicationError("component too large");
          changes.set(id, r.take(Number(len - 1n)));
        }
      }
      const e = this.entities.get(last);
      const lastFrame = this.datagramFrames.get(last);
      const current =
        usable &&
        e !== undefined &&
        this.generations.get(last) === generation &&
        (lastFrame === undefined || frame > lastFrame);
      if (!current || !e) {
        summary.skipped += 1;
        continue;
      }
      if (qx !== null) e.qx = qx;
      if (qy !== null) e.qy = qy;
      if (qz !== null) e.qz = qz;
      e.x = e.qx * this.precision;
      e.y = e.qy * this.precision;
      e.z = e.qz * this.precision;
      for (const id of removed) e.components.delete(id);
      for (const [id, bytes] of changes) e.components.set(id, bytes);
      this.datagramFrames.set(last, frame);
      summary.updated.push(last);
    }
    if (!r.done) throw new ReplicationError("trailing bytes");
    return summary;
  }
}
