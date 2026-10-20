// Offline photo uploads (`queueUpload`): the file waits on the device, is
// uploaded when there is a network, and is attached to its record with an
// ordinary queued write.

import "fake-indexeddb/auto";
import { afterEach, describe, expect, test } from "bun:test";

import {
  IndexedDBPersistence,
  IndexedDBUploadStore,
  SyncEngine,
  UploadQueueFullError,
  UploadRejectedError,
} from "./index";
import { createTestEnv, type TestEnv } from "./test-harness";

const photo = (bytes = 1024, name = "hail.jpg") =>
  new File([new Uint8Array(bytes)], name, { type: "image/jpeg" });

const target = { entity: "Photo", id: "p1", field: "fileId", urlField: "url" };

async function until(cond: () => boolean, ms = 2000) {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 10));
  }
}

describe("queueUpload", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  async function start(opts: { maxQueuedUploadBytes?: number } = {}) {
    env = createTestEnv({ transport: "poll" });
    if (opts.maxQueuedUploadBytes !== undefined) {
      (env.engine as unknown as { uploads: { maxBytes: number } }).uploads.maxBytes =
        opts.maxQueuedUploadBytes;
    }
    env.signIn({ userId: "rep1" });
    env.server.seed("Photo", [{ id: "p1", leadId: "l1", fileId: null, url: null }]);
    await env.start();
    return env;
  }

  test("the record shows the local copy at once, then the uploaded file", async () => {
    const e = await start();
    const queued = await e.engine.queueUpload(photo(), { attach: target });
    expect(queued.ref).toBe(`local:${queued.localId}`);
    // At once, from the device.
    expect(e.engine.store.get("Photo", "p1")).toMatchObject({ fileId: queued.ref, url: queued.ref });

    const file = await queued.done;
    expect(file).toMatchObject({ id: "f_1", url: "/api/files/f_1", size: 1024 });
    await until(() => e.engine.pendingCount() === 0);
    expect(e.engine.store.get("Photo", "p1")).toMatchObject({ fileId: "f_1", url: "/api/files/f_1" });
    const [row] = await e.server.listEntityRows("Photo", e.token);
    expect(row).toMatchObject({ fileId: "f_1", url: "/api/files/f_1" });
    expect(e.server.files.stored.get("f_1")).toEqual({ filename: "hail.jpg", size: 1024, owner: "rep1" });
    expect(e.engine.outbox()).toEqual({ pending: [], failed: [] });
  });

  test("a rejected upload stays failed and is not retried in a loop", async () => {
    const e = await start();
    e.server.files.rejectInit(415, "FILE_TYPE_NOT_ALLOWED", "image/heic is not allowed");
    const queued = await e.engine.queueUpload(photo(), { attach: target });
    await expect(queued.done).rejects.toBeInstanceOf(UploadRejectedError);
    await expect(queued.done).rejects.toMatchObject({ code: "FILE_TYPE_NOT_ALLOWED" });

    // Reconnects and later processing do not resend it.
    await e.engine.uploads.process();
    await e.engine.uploads.process();
    expect(e.server.files.initCount).toBe(1);

    const [failed] = e.engine.outbox().failed;
    expect(failed).toMatchObject({
      id: queued.localId,
      kind: "upload",
      entity: "Photo",
      rowId: "p1",
      error: "image/heic is not allowed",
      errorCode: "FILE_TYPE_NOT_ALLOWED",
      data: { field: "fileId", urlField: "url", filename: "hail.jpg", size: 1024 },
    });

    // Retry sends it again; dismiss drops it.
    e.server.files.clearRejection();
    await e.engine.retryWrite(queued.localId);
    await until(() => e.engine.outbox().failed.length === 0 && e.engine.outbox().pending.length === 0);
    expect(e.server.files.stored.size).toBe(1);

    e.server.files.rejectInit(413, "FILE_TOO_LARGE", "too large");
    const second = await e.engine.queueUpload(photo(), { attach: target });
    await second.done.catch(() => {});
    expect(e.engine.dismissWrite(second.localId)).toBe(true);
    expect(e.engine.outbox().failed).toHaveLength(0);
    expect(e.engine.uploadLocalUrl(second.localId)).toBeNull();
  });

  test("a connection dropped mid-PUT keeps the upload pending and it is sent later", async () => {
    const e = await start();
    e.server.files.dropNextPut();
    const queued = await e.engine.queueUpload(photo(), { attach: target });
    await until(() => e.server.files.putCount === 1);
    await e.flush(20);
    expect(e.engine.outbox().pending.map((p) => p.id)).toEqual([queued.localId]);
    expect(e.engine.outbox().failed).toHaveLength(0);

    // The next attempt (a reconnect, or the backoff timer).
    await e.engine.uploads.process();
    const file = await queued.done;
    expect(file.id).toBe("f_2");
    expect(e.server.files.stored.has("f_2")).toBe(true);
  });

  test("an upload queued as one user is discarded when another signs in", async () => {
    const e = await start();
    // Offline: init fails like no network.
    e.server.files.dropNextPut();
    const queued = await e.engine.queueUpload(photo(), { attach: target });
    await until(() => e.server.files.putCount === 1);

    e.signIn({ userId: "rep2" });
    await e.engine.refreshResolvedSession();
    await expect(queued.done).rejects.toMatchObject({ code: "DISCARDED" });
    expect(e.engine.outbox().pending.filter((p) => p.kind === "upload")).toHaveLength(0);
    expect(e.server.files.stored.size).toBe(0);
  });

  test("past the size limit queueUpload refuses with a clear error", async () => {
    const e = await start({ maxQueuedUploadBytes: 3000 });
    e.server.files.dropNextPut();
    await e.engine.queueUpload(photo(2000), { attach: target });
    let caught: unknown;
    try {
      await e.engine.queueUpload(photo(2000), { attach: target });
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(UploadQueueFullError);
    expect((caught as UploadQueueFullError).code).toBe("UPLOAD_QUEUE_FULL");
  });
});

describe("queued uploads survive a reload", () => {
  const realFetch = globalThis.fetch;
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  test("the file and its owner are kept, and it uploads on the next start", async () => {
    const name = `uploads-${Math.random().toString(36).slice(2)}`;
    // Run 1, offline: queue a photo.
    globalThis.fetch = (async () => {
      throw new TypeError("Failed to fetch");
    }) as unknown as typeof fetch;
    const disk = new IndexedDBPersistence(name);
    await disk.open();
    await disk.saveIdentity("rep1");
    await disk.saveRow("Photo", "p1", { id: "p1", fileId: null, url: null });
    disk.connection?.close();

    const first = new SyncEngine({
      baseUrl: "http://test.invalid",
      appName: name,
      persistence: new IndexedDBPersistence(name),
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
    });
    const started = first.start().catch(() => {});
    await until(() => first.isHydrated());
    await new Promise((r) => setTimeout(r, 20));
    const queued = await first.queueUpload(photo(4096, "roof.jpg"), { attach: target });
    first.stop();
    await started;
    (first as unknown as { persistence: IndexedDBPersistence }).persistence.connection?.close();

    const store = new IndexedDBUploadStore(name);
    const [saved] = await store.list();
    expect(saved).toMatchObject({
      localId: queued.localId,
      owner: "rep1",
      filename: "roof.jpg",
      size: 4096,
      status: "pending",
      attach: target,
    });
    expect(saved!.blob.size).toBe(4096);
    await store.close();

    // Run 2, online.
    globalThis.fetch = realFetch;
    const env = createTestEnv({ transport: "poll", appName: name });
    env.signIn({ userId: "rep1" });
    env.server.seed("Photo", [{ id: "p1", fileId: null, url: null }]);
    const second = new SyncEngine({
      baseUrl: "http://test.invalid",
      appName: name,
      persistence: new IndexedDBPersistence(name),
      storage: (env.engine as unknown as { storage: SyncEngine["storage"] }).storage,
      multiTab: false,
      transport: "poll",
      pollInterval: 1e9,
    });
    try {
      await second.start();
      await until(() => env.server.files.stored.size === 1);
      await until(() => second.pendingCount() === 0);
      const [row] = await env.server.listEntityRows("Photo", env.token);
      expect(row).toMatchObject({ fileId: "f_1", url: "/api/files/f_1" });
      expect(await new IndexedDBUploadStore(name).list()).toEqual([]);
    } finally {
      second.stop();
      (second as unknown as { persistence: IndexedDBPersistence }).persistence.connection?.close();
      await env.dispose();
    }
  });
});
