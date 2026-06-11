/**
 * Regression tests for the ctx.db builder surfaces (runtime.ts).
 *
 * Guards the contract the types now promise (types.ts made `unsafe`
 * REQUIRED on the top-level db and absent on the unsafe surface):
 *
 *   1. `db.unsafe` always exists, with the full op surface.
 *   2. `db.unsafe.unsafe` does NOT exist (no chaining — it would be a
 *      runtime undefined that the old optional type silently allowed).
 *   3. Plain ops emit `unsafe_op: false`; `unsafe.*` ops emit
 *      `unsafe_op: true` — the flag the Rust policy gate keys on.
 *
 * runtime.ts runs main() on import (it IS the bun runner entrypoint),
 * so the builders are exercised in a child process: we feed it a
 * script over a kept-open stdin pipe and read the NDJSON protocol
 * frames it writes to stdout.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

const SCRIPT = `
import { buildDbReader, buildDbWriter } from ${JSON.stringify(RUNTIME)};

const reader = buildDbReader("c_r");
const writer = buildDbWriter("c_w");

// Structural facts ride stderr (fenceStdout reserves stdout for frames).
console.error("STRUCT " + JSON.stringify({
  readerUnsafe: typeof reader.unsafe.get === "function",
  writerUnsafeWrites: typeof (writer.unsafe as any).update === "function",
  noReaderChain: (reader.unsafe as any).unsafe === undefined,
  noWriterChain: (writer.unsafe as any).unsafe === undefined,
}));

// Fire one op per surface — no host replies, so don't await; the
// emitted frames on stdout are the assertion target.
reader.get("Todo", "r1").catch(() => {});
reader.unsafe.get("Todo", "r2").catch(() => {});
writer.update("Todo", "w1", { a: 1 }).catch(() => {});
writer.unsafe.update("Todo", "w2", { a: 2 }).catch(() => {});

setTimeout(() => process.exit(0), 300);
`;

test("db builders: required unsafe surface, no chaining, unsafe_op flag routing", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-db-"));
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, SCRIPT);

  const proc = Bun.spawn(["bun", scriptPath], {
    stdin: "pipe", // keep main()'s readerLoop alive until the probe exits itself
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);

  // 1+2: structure as reported from inside the child.
  const structLine = stderr.split("\n").find((l) => l.includes("STRUCT "));
  expect(structLine).toBeDefined();
  const struct = JSON.parse(structLine!.slice(structLine!.indexOf("STRUCT ") + 7));
  expect(struct.readerUnsafe).toBe(true);
  expect(struct.writerUnsafeWrites).toBe(true);
  expect(struct.noReaderChain).toBe(true);
  expect(struct.noWriterChain).toBe(true);

  // 3: every emitted db frame carries the right unsafe_op flag.
  const frames = stdout
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, unknown>)
    .filter((f) => f.type === "db");
  const byId = (id: string) => frames.find((f) => f.id === id);

  expect(byId("r1")).toMatchObject({ op: "get", unsafe_op: false });
  expect(byId("r2")).toMatchObject({ op: "get", unsafe_op: true });
  expect(byId("w1")).toMatchObject({ op: "update", unsafe_op: false });
  expect(byId("w2")).toMatchObject({ op: "update", unsafe_op: true });
});
