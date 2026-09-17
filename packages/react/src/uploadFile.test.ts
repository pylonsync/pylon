// `uploadFile` used to POST to /api/files/upload, which the runtime removed in
// 0.3.91 and answers with 410, so every `<FileUpload>` failed. These tests pin
// the three-step flow and the one credential rule that matters: the session
// goes to pylon, never to a presigned storage URL on another origin.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { getBaseUrl, uploadFile } from "./index";

type Seen = { url: string; method: string; headers: Record<string, string>; body: unknown };

describe("uploadFile", () => {
  const realFetch = globalThis.fetch;
  const realWindow = (globalThis as { window?: unknown }).window;
  let seen: Seen[];
  let uploadUrl: string;
  // Whatever origin the client resolves to. Another test file in the same
  // process may have configured one, and this test must not set one itself:
  // configureClient is process-wide and would leak into those tests.
  let base: string;

  beforeEach(() => {
    seen = [];
    uploadUrl = "/api/files/local-put/f_1";
    (globalThis as { window?: unknown }).window = { location: { origin: "https://app.example" } };
    base = getBaseUrl();
    globalThis.fetch = (async (input: any, init: any = {}) => {
      const url = String(input?.url ?? input);
      seen.push({ url, method: init.method ?? "GET", headers: (init.headers ?? {}) as Record<string, string>, body: init.body });
      const json = (value: unknown) => new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json" } });
      if (url.endsWith("/api/files/init")) return json({ uploadUrl, assetId: "f_1" });
      if (url.endsWith("/api/files/confirm")) return json({ id: "f_1", url: "/api/files/f_1", size: 5 });
      return new Response(null, { status: 200 });
    }) as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = realFetch;
    (globalThis as { window?: unknown }).window = realWindow;
  });

  test("init, put, confirm, with the visibility on init", async () => {
    const file = new File(["hello"], "cat.png", { type: "image/png" });
    const out = await uploadFile(file, { visibility: "public", token: "tok" });
    expect(out).toEqual({ id: "f_1", url: "/api/files/f_1", size: 5 });
    expect(seen.map((r) => `${r.method} ${r.url}`)).toEqual([
      `POST ${base}/api/files/init`,
      `PUT ${base}/api/files/local-put/f_1`,
      `POST ${base}/api/files/confirm`,
    ]);
    expect(JSON.parse(String(seen[0].body))).toEqual({ filename: "cat.png", mimeType: "image/png", size: 5, visibility: "public" });
    expect(seen[1].headers.Authorization).toBe("Bearer tok");
    expect(seen[1].headers["Content-Type"]).toBe("image/png");
    expect(JSON.parse(String(seen[2].body))).toEqual({ assetId: "f_1" });
  });

  test("defaults to private", async () => {
    await uploadFile(new Uint8Array([1, 2, 3]), { token: "tok" });
    expect(JSON.parse(String(seen[0].body))).toEqual({ filename: "upload", mimeType: "application/octet-stream", size: 3, visibility: "private" });
  });

  test("a presigned URL on another origin gets no session", async () => {
    uploadUrl = "https://bucket.s3.example/f_1?X-Amz-Signature=abc";
    await uploadFile(new Blob(["hi"], { type: "text/plain" }), { token: "tok" });
    expect(seen[1].url).toBe("https://bucket.s3.example/f_1?X-Amz-Signature=abc");
    expect(seen[1].headers.Authorization).toBeUndefined();
  });

  test("a refused PUT throws before confirm", async () => {
    globalThis.fetch = (async (input: any, init: any = {}) => {
      const url = String(input?.url ?? input);
      seen.push({ url, method: init.method ?? "GET", headers: {}, body: init.body });
      if (url.endsWith("/api/files/init")) return new Response(JSON.stringify({ uploadUrl, assetId: "f_1" }), { status: 200 });
      return new Response("nope", { status: 404 });
    }) as typeof fetch;
    await expect(uploadFile(new Uint8Array([1]), { token: "tok" })).rejects.toThrow("failed: 404");
    expect(seen.some((r) => r.url.endsWith("/api/files/confirm"))).toBe(false);
  });
});
