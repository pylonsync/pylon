// `useQuery` / `useQueryOne` expose `synced`: false until the engine has
// completed a server pull in this session. Without it an app could not tell
// a warm but incomplete local replica from a complete one — `loading` is
// false as soon as cached rows exist (saas template showed stale partial
// data briefly).

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useQuery, useQueryOne } from "./hooks";

const asEngine = (e: TestEnv["engine"]): SyncEngine => e as unknown as SyncEngine;

let env: TestEnv | null = null;
afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

function Probe({ engine }: { engine: SyncEngine }) {
  const list = useQuery<{ id: string }>(engine, "Todo");
  const one = useQueryOne<{ id: string }>(engine, "Todo", "t1");
  return (
    <div>
      <span data-testid="list">{`${list.data.length}:${list.loading}:${list.synced}`}</span>
      <span data-testid="one">{`${one.data ? "row" : "none"}:${one.synced}`}</span>
    </div>
  );
}

describe("useQuery synced", () => {
  test("cached rows render with synced false until the pull confirms them", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    env = createTestEnv({ transport: "poll", beforePull: () => gate });
    env.signIn({ userId: "u1" });
    env.server.seed("Todo", [{ id: "t1" }, { id: "t2" }]);
    // A row the local replica already holds (what a warm cache provides).
    env.engine.store.applyChange({
      seq: 0,
      entity: "Todo",
      row_id: "t1",
      kind: "insert",
      data: { id: "t1" },
      timestamp: "",
    });

    render(<Probe engine={asEngine(env.engine)} />);
    expect(screen.getByTestId("list").textContent).toBe("1:false:false");
    expect(screen.getByTestId("one").textContent).toBe("row:false");

    let started!: Promise<void>;
    act(() => {
      started = env!.engine.start();
    });
    release();
    await act(async () => {
      await started;
    });
    await waitFor(() =>
      expect(screen.getByTestId("list").textContent).toBe("2:false:true"),
    );
    expect(screen.getByTestId("one").textContent).toBe("row:true");
  });
});
