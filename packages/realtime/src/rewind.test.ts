import { expect, test } from "bun:test";

import { EntityInterpolator } from "./interpolation";
import { EntityTable } from "./replication";
import fixtures from "./rewind.fixtures.json";

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

// The server's lag compensation (pylon-replication's ViewHistory) places
// each entity where this interpolator does, for the frames it sent:
// crates/realtime/tests/rewind_fixtures.rs writes the frames and the
// server's positions.
test("the interpolator draws what the server's rewind says it drew", () => {
  const table = new EntityTable();
  const interp = new EntityInterpolator();
  for (const f of fixtures.frames) {
    interp.record(table, table.apply(bytes(f.frame)), f.tick);
  }
  expect(fixtures.queries.length).toBeGreaterThan(40);
  for (const q of fixtures.queries) {
    interp.update(q.tick);
    const want = new Map(q.positions.map(([id, x, y, z]) => [id, [x, y, z]]));
    const got = new Map([...interp.entities].map(([id, e]) => [id, [e.x, e.y, e.z]]));
    expect([...got.keys()].sort((a, b) => a - b)).toEqual([...want.keys()].sort((a, b) => a - b));
    for (const [id, p] of want) expect(got.get(id)).toEqual(p);
  }
});
