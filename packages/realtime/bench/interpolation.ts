// Run from the repository root: bun packages/realtime/bench/interpolation.ts
import { EntityInterpolator } from "../src/interpolation";
import { EntityTable } from "../src/replication";

for (const batch of [20, 200]) {
  const table = new EntityTable();
  const interp = new EntityInterpolator();
  const times: number[] = [];
  let previous: number[] = [];
  // Keep recording while rendering is paused, as in a hidden tab.
  for (let tick = 1; tick <= 1200; tick++) {
    table.entities.clear();
    const spawned: number[] = [];
    for (let i = 0; i < batch; i++) {
      const id = tick * batch + i;
      table.entities.set(id, { id, qx: 0, qy: 0, qz: 0, x: 0, y: 0, z: 0, components: new Map() });
      spawned.push(id);
    }
    const start = performance.now();
    interp.record(table, { full: false, spawned, despawned: previous, updated: [] }, tick);
    const elapsed = performance.now() - start;
    if (tick > 400) times.push(elapsed);
    previous = spawned;
  }
  interp.update(1200);
  if (interp.entities.size !== batch) throw new Error("unexpected final entity count");
  times.sort((a, b) => a - b);
  console.log(`${batch} spawns and despawns/tick: ${(times[times.length / 2] * 1000).toFixed(2)} us median`);
}
