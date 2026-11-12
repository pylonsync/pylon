import { expect, test } from "bun:test";

// Execute the published examples with a database that does not filter rows.
for (const path of ["./types.ts", "../../../apps/docs/concepts/functions.mdx"]) {
  const source = await Bun.file(new URL(path, import.meta.url)).text();
  const example = source.match(/export default query\(\{[\s\S]*?ctx\.files\.signedUrl[\s\S]*?\n(?: \* )?\}\);/);
  if (!example) throw new Error(`Missing signed file example in ${path}`);
  const code = new Bun.Transpiler({ loader: "ts" }).transformSync(
    `import { query, v } from ${JSON.stringify(new URL("./index.ts", import.meta.url).pathname)};\n` +
      example[0].replace(/^ \* ?/gm, ""),
  );
  const { default: fn } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);

  function fixture(member = true, exists = true) {
    const signed: string[] = [];
    const ctx = {
      db: { get: async () => exists ? { orgId: "org-a", fileId: "file-a" } : null },
      requireMember: async (orgId: string) => {
        if (!member || orgId !== "org-a") throw Object.assign(new Error("Denied"), { code: "FORBIDDEN" });
      },
      files: { signedUrl: async (fileId: string) => { signed.push(fileId); return `/signed/${fileId}`; } },
      error: (code: string, message: string) => Object.assign(new Error(message), { code }),
    };
    return { ctx, signed };
  }

  test(`${path}: a member cannot sign an unrelated private file`, async () => {
    const { ctx, signed } = fixture();
    await expect(fn.handler(ctx, { eventId: "event-a", fileId: "file-b" })).rejects.toMatchObject({ code: "NOT_FOUND" });
    expect(signed).toEqual([]);
  });

  test(`${path}: an organizer can sign the event file`, async () => {
    const { ctx, signed } = fixture();
    expect(await fn.handler(ctx, { eventId: "event-a", fileId: "file-a" })).toEqual({ url: "/signed/file-a" });
    expect(signed).toEqual(["file-a"]);
  });

  test(`${path}: outsiders and missing events cannot sign files`, async () => {
    for (const [member, exists] of [[false, true], [true, false]]) {
      const { ctx, signed } = fixture(member, exists);
      await expect(fn.handler(ctx, { eventId: "event-a", fileId: "file-a" })).rejects.toMatchObject({ code: exists ? "FORBIDDEN" : "NOT_FOUND" });
      expect(signed).toEqual([]);
    }
  });
}
