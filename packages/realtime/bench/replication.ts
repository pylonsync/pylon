// Run from the repository root: bun packages/realtime/bench/replication.ts
import { EntityTable } from "../src/replication";
import { replicationPayload } from "../test/frames";

function run(entities: number, updates: number): void {
  const table = new EntityTable();
  table.apply(replicationPayload({
    full: true,
    spawn: Array.from({ length: entities }, (_, id) => ({ id })),
  }));
  const frames = [1, -1].map((x) => replicationPayload({
    update: Array.from({ length: updates }, (_, id) => ({
      id,
      pos: [x, 0, 0] as [number, number, number],
    })),
  }));
  for (let i = 0; i < 2000; i++) table.apply(frames[i % 2]);
  const times: number[] = [];
  const iterations = 1000;
  for (let sample = 0; sample < 9; sample++) {
    const start = performance.now();
    for (let i = 0; i < iterations; i++) table.apply(frames[i % 2]);
    times.push((performance.now() - start) * 1000 / iterations);
  }
  times.sort((a, b) => a - b);
  if (table.get(0)?.x !== 0) throw new Error("position drift");
  console.log(`${entities} entities, ${updates} updates: ${times[4].toFixed(2)} us/frame (median batch)`);
}

run(100_000, 0);
run(100_000, 1);
run(100_000, 100);
run(1_000, 1_000);
