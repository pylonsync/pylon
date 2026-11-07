import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmdirSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const dir = mkdtempSync(join(tmpdir(), "pylon-ai-studio-security-"));
const modules: string[] = [];

async function prepareTemplate(name: string) {
  const root = new URL("../../create-pylon/templates/ai-studio/", import.meta.url);
  const source = (await Bun.file(new URL(`functions/${name}.ts`, root)).text())
    .replace('"@pylonsync/functions"', JSON.stringify(new URL("./index.ts", import.meta.url).pathname))
    .replace('"../lib/studio"', JSON.stringify(new URL("lib/studio.ts", root).pathname));
  const path = join(dir, `${name}.ts`);
  await Bun.write(path, source);
  modules.push(path);
  return path;
}

const paths = await Promise.all(["pollGeneration", "_getGeneration", "_updateGeneration"].map(prepareTemplate));
const [poll, get, update] = await Promise.all(paths.map(async path => (await import(path)).default));

afterAll(() => {
  for (const path of modules) unlinkSync(path);
  rmdirSync(dir);
});

function fixture(userId: string | null, status = "pending") {
  const row: Record<string, unknown> = {
    id: "generation-a", userId: "owner", kind: "image", prompt: "A tree", status,
  };
  const writes: unknown[] = [];
  const jobs: unknown[] = [];
  const db = { unsafe: {
    get: async (_entity: string, id: string) => id === row.id ? row : null,
    update: async (_entity: string, _id: string, patch: Record<string, unknown>) => {
      writes.push(patch);
      Object.assign(row, patch);
    },
  } };
  const ctx = {
    auth: { userId }, env: {},
    error: (code: string, message: string) => Object.assign(new Error(message), { code }),
    runQuery: async (name: string, args: unknown) => {
      expect(name).toBe("_getGeneration");
      return get.handler({ db }, args);
    },
    runMutation: async (name: string, args: unknown) => {
      expect(name).toBe("_updateGeneration");
      return update.handler({ db }, args);
    },
    scheduler: { runAfter: async (...args: unknown[]) => { jobs.push(args); } },
  };
  return { ctx, row, writes, jobs };
}

for (const userId of [null, "other-user"]) {
  test(`poll refuses ${userId ?? "anonymous"} before any generation change`, async () => {
    const { ctx, row, writes, jobs } = fixture(userId);
    await expect(poll.handler(ctx, { id: row.id, attempts: 201 })).rejects.toMatchObject({
      code: userId ? "NOT_FOUND" : "AUTH_REQUIRED",
    });
    expect(row.status).toBe("pending");
    expect(writes).toEqual([]);
    expect(jobs).toEqual([]);
  });
}

test("the generation owner can complete a scheduled demo job", async () => {
  const { ctx, row, writes } = fixture("owner");
  expect(await poll.handler(ctx, { id: row.id })).toEqual({ ok: true });
  expect(row.status).toBe("done");
  expect(row.demo).toBe(true);
  expect(writes).toHaveLength(1);
});

test("the generation owner can end an expired job", async () => {
  const { ctx, row } = fixture("owner");
  await poll.handler(ctx, { id: row.id, attempts: 201 });
  expect(row.status).toBe("failed");
});

test("a completed job does not change on retry", async () => {
  const { ctx, row, writes, jobs } = fixture("owner", "done");
  await poll.handler(ctx, { id: row.id });
  expect(writes).toEqual([]);
  expect(jobs).toEqual([]);
});

test("HTTP polling requires a user session", () => {
  expect(poll.auth).toBe("user");
});
