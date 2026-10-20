// `useOutbox` re-renders as writes go pending, reach the server, or fail, and
// its retry/dismiss act on the failed list.

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useOutbox } from "./useOutbox";

const asEngine = (e: TestEnv["engine"]): SyncEngine => e as unknown as SyncEngine;

let env: TestEnv | null = null;
afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

let latest: ReturnType<typeof useOutbox> | null = null;
function Probe({ engine }: { engine: SyncEngine }) {
  const outbox = useOutbox(engine);
  latest = outbox;
  return (
    <span data-testid="outbox">
      {`${outbox.pending.length} not synced, ${outbox.failed.length} failed` +
        (outbox.failed[0] ? `: ${outbox.failed[0].error}` : "")}
    </span>
  );
}

const text = () => screen.getByTestId("outbox").textContent;

describe("useOutbox", () => {
  test("re-renders when a write goes pending, then reaches the server", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    render(<Probe engine={asEngine(env.engine)} />);
    expect(text()).toBe("0 not synced, 0 failed");

    env.server.primeNextPushOutcome({ kind: "network" });
    await act(async () => {
      await env!.engine.insert("Knock", { door: "1" });
    });
    expect(text()).toBe("1 not synced, 0 failed");

    await act(async () => {
      await env!.engine.push();
    });
    await waitFor(() => expect(text()).toBe("0 not synced, 0 failed"));
  });

  test("re-renders when a write fails; retry and dismiss work from the hook", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.rejectWritesWhere("Knock", (d) => d.door === "", "VALIDATION_FAILED", "door number is required");
    render(<Probe engine={asEngine(env.engine)} />);

    await act(async () => {
      await env!.engine.insert("Knock", { door: "" }).catch(() => {});
    });
    expect(text()).toBe("0 not synced, 1 failed: door number is required");

    await act(async () => {
      await latest!.retry(latest!.failed[0]!.id, { door: "7" });
    });
    await waitFor(() => expect(text()).toBe("0 not synced, 0 failed"));

    await act(async () => {
      await env!.engine.insert("Knock", { door: "" }).catch(() => {});
    });
    expect(text()).toBe("0 not synced, 1 failed: door number is required");
    act(() => {
      expect(latest!.dismiss(latest!.failed[0]!.id)).toBe(true);
    });
    expect(text()).toBe("0 not synced, 0 failed");
  });
});
