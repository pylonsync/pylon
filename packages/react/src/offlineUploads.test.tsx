// A queued photo shows on its record at once, from the device, and the
// record shows the uploaded file's URL after the upload runs.

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useQueryOne } from "./hooks";
import { queueUpload, useFileUrl } from "./offlineUploads";

const asEngine = (e: TestEnv["engine"]): SyncEngine => e as unknown as SyncEngine;

let env: TestEnv | null = null;
afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

function PhotoCard({ engine }: { engine: SyncEngine }) {
  const { data } = useQueryOne<{ id: string; url: string | null }>(engine, "Photo", "p1");
  const src = useFileUrl(data?.url, engine);
  return <span data-testid="src">{src ?? "none"}</span>;
}

describe("queueUpload + useFileUrl", () => {
  test("the record shows the local photo at once and the real URL after the upload", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "rep1" });
    env.server.seed("Photo", [{ id: "p1", url: null, fileId: null }]);
    await env.start();
    const engine = asEngine(env.engine);
    render(<PhotoCard engine={engine} />);
    expect(screen.getByTestId("src").textContent).toBe("none");

    // Hold the upload so the local stage is observable.
    env.server.files.dropNextPut();
    let queued!: Awaited<ReturnType<typeof queueUpload>>;
    await act(async () => {
      queued = await queueUpload(new File([new Uint8Array(64)], "door.jpg", { type: "image/jpeg" }), {
        attach: { entity: "Photo", id: "p1", field: "fileId", urlField: "url" },
        engine,
      });
    });
    expect(queued.localUrl).toMatch(/^blob:/);
    expect(screen.getByTestId("src").textContent).toBe(queued.localUrl!);

    await act(async () => {
      await env!.engine.uploads.process();
      await queued.done;
    });
    await waitFor(() => expect(screen.getByTestId("src").textContent).toBe("/api/files/f_2"));
  });

  test("a local reference this device does not hold resolves to null", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "rep1" });
    env.server.seed("Photo", [{ id: "p1", url: "local:upl_from_another_phone", fileId: null }]);
    await env.start();
    render(<PhotoCard engine={asEngine(env.engine)} />);
    await waitFor(() => expect(screen.getByTestId("src").textContent).toBe("none"));
  });
});
