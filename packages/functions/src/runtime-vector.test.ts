/**
 * Wire-shape tests for the vector surfaces (runtime.ts):
 *
 *   1. `ctx.db.vectorSearch` emits `{type:"db", op:"vector_search"}`
 *      frames with the query on `data` and the unsafe_op / ssr_read
 *      flags the Rust policy gate keys on.
 *   2. `ctx.llm.embed` emits `{type:"llm_embed", request:{input,model}}`
 *      with an op_id so concurrent embeds demux.
 *
 * Same child-process harness as runtime-db.test.ts — runtime.ts runs
 * main() on import, so the builders are exercised in a probe script
 * whose emitted NDJSON frames are the assertion target.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

const SCRIPT = `
import { buildDbReader, buildLlm } from ${JSON.stringify(RUNTIME)};

const reader = buildDbReader("c_r");
const llm = buildLlm("c_l");

// No host replies — don't await; the emitted frames are the target.
reader.vectorSearch("Doc", {
  field: "embedding",
  vector: [1, 0, 0],
  limit: 5,
  metric: "l2",
  filter: { kind: "a" },
}).catch(() => {});
reader.unsafe.vectorSearch("Doc", { field: "embedding", vector: [1] }).catch(() => {});
llm.embed(["hello", "world"], { model: "text-embedding-3-large" }).catch(() => {});

setTimeout(() => process.exit(0), 300);
`;

test("vectorSearch + llm.embed emit the exact wire shapes the host parses", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-vec-"));
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, SCRIPT);

  const proc = Bun.spawn(["bun", scriptPath], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout] = await Promise.all([
    new Response(proc.stdout).text(),
    proc.exited,
  ]);

  const frames = stdout
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as Record<string, any>);

  const vs = frames.filter((f) => f.type === "db" && f.op === "vector_search");
  expect(vs.length).toBe(2);
  const safe = vs.find((f) => f.unsafe_op === false)!;
  expect(safe).toMatchObject({
    entity: "Doc",
    ssr_read: false,
    data: {
      field: "embedding",
      vector: [1, 0, 0],
      limit: 5,
      metric: "l2",
      filter: { kind: "a" },
    },
  });
  expect(safe.op_id).toBeDefined();
  const unsafe = vs.find((f) => f.unsafe_op === true)!;
  expect(unsafe.data.field).toBe("embedding");

  const embed = frames.find((f) => f.type === "llm_embed")!;
  expect(embed).toBeDefined();
  expect(embed.call_id).toBe("c_l");
  expect(embed.op_id).toBeDefined();
  expect(embed.request).toMatchObject({
    input: ["hello", "world"],
    model: "text-embedding-3-large",
  });
});
