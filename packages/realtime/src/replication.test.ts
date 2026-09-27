import { describe, expect, test } from "bun:test";

import { EntityTable, ReplicationError } from "./replication";
import fixtures from "./replication.fixtures.json";

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function hex(b: Uint8Array): string {
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

describe("the Rust encoder's frames (replication.fixtures.json)", () => {
  test("apply to the same tables the Rust decoder holds", () => {
    const table = new EntityTable();
    for (const [i, step] of fixtures.frames.entries()) {
      table.apply(bytes(step.frame));
      const got = [...table.entities.values()]
        .sort((a, b) => a.id - b.id)
        .map((e) => ({
          id: e.id,
          q: [e.qx, e.qy, e.qz],
          components: Object.fromEntries(
            [...e.components.entries()].map(([k, v]) => [String(k), hex(v)]),
          ),
        }));
      expect(got, `after frame ${i}`).toEqual(step.table as unknown as typeof got);
      for (const e of table.entities.values()) {
        expect(e.x).toBeCloseTo(e.qx * step.precision, 9);
      }
    }
  });
});

describe("the Rust encoder's datagrams (replication.fixtures.json)", () => {
  test("apply, and skip, exactly as the Rust table does", () => {
    const table = new EntityTable();
    const tableJson = () =>
      [...table.entities.values()]
        .sort((a, b) => a.id - b.id)
        .map((e) => ({
          id: e.id,
          q: [e.qx, e.qy, e.qz],
          components: Object.fromEntries(
            [...e.components.entries()].map(([k, v]) => [String(k), hex(v)]),
          ),
        }));
    for (const [i, event] of fixtures.datagrams.entries()) {
      if ("stream" in event && event.stream) {
        table.apply(bytes(event.stream), event.tick as number);
      } else if ("datagram" in event && event.datagram) {
        const s = table.applyDatagram(bytes(event.datagram));
        expect(s.frame, `event ${i}`).toBe(event.frame);
        expect(s.tick).toBe(event.tick);
        expect(s.ack).toBe(event.ack);
        expect(s.streamTick).toBe(event.sentStreamTick);
        expect(s.parts).toBe(event.parts);
        expect(s.updated, `event ${i}`).toEqual(event.updated);
        expect(s.skipped, `event ${i}`).toBe(event.skipped);
        expect(table.streamTick).toBe(event.streamTick);
      }
      expect(tableJson(), `after event ${i}`).toEqual(event.table as unknown as ReturnType<typeof tableJson>);
    }
  });

  test("hostile datagrams are refused, not half-applied", () => {
    const t = new EntityTable();
    expect(() => t.applyDatagram(new Uint8Array())).toThrow(ReplicationError);
    expect(() => t.applyDatagram(new Uint8Array([1]))).toThrow("datagram version 1");
    // version, frame 1, tick 1, ack 0, stream tick 1, precision 1.0,
    // parts 1, count 1, id 0, then nothing.
    expect(() =>
      t.applyDatagram(new Uint8Array([2, 1, 1, 0, 1, 0, 0, 0x80, 0x3f, 1, 1, 0])),
    ).toThrow("ends early");
    expect(() =>
      t.applyDatagram(new Uint8Array([2, 1, 1, 0, 1, 0, 0, 0x80, 0x3f, 1, 0, 7])),
    ).toThrow("trailing bytes");
  });
});

describe("hostile frames", () => {
  const head = (...rest: number[]) => {
    const f = new Uint8Array(6 + rest.length);
    f[0] = 1;
    new DataView(f.buffer).setFloat32(2, 1, true);
    f.set(rest, 6);
    return f;
  };

  test("are refused, not half-applied into garbage", () => {
    const t = new EntityTable();
    expect(() => t.apply(new Uint8Array())).toThrow(ReplicationError);
    expect(() => t.apply(new Uint8Array([2, 0, 0, 0, 0x80, 0x3f]))).toThrow("version 2");
    expect(() => t.apply(new Uint8Array([1, 0, 0, 0, 0, 0]))).toThrow("bad precision");
    expect(() => t.apply(head(0xff, 0xff, 0xff, 0xff, 0x0f))).toThrow("count too large");
    expect(() => t.apply(head(0, 0, 1, 7, 0))).toThrow("unknown entity 7");
    expect(() => t.apply(head(2, 5, 0, 0, 0))).toThrow("do not ascend");
    expect(() => t.apply(head(0, 0, 0, 9))).toThrow("trailing bytes");
    expect(() => t.apply(head(0, 1, 1, 0, 0, 0, 1, 4, 50, 1))).toThrow("runs past the end");
  });

  test("an id beyond 2^53 is refused rather than rounded", () => {
    // One spawn with id 2^53 (varint 80 80 80 80 80 80 80 10).
    const f = head(0, 1, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10, 0, 0, 0, 0, 0);
    expect(() => new EntityTable().apply(f)).toThrow("beyond 2^53");
  });
});
