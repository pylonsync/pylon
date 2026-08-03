// Regression coverage for `useQuery` / `useQueryOne` loading state, mounted.
//
// The bug: `loading` was a `useRef` flipped inside `getSnapshot`. React's
// useSyncExternalStore only re-renders when getSnapshot returns a value that
// isn't Object.is-equal to the previous one — and the snapshot for an entity
// with NO rows is the same cached array before and after the initial pull
// settles. So markInitialSyncSettled() notified, getSnapshot ran, the ref
// flipped to false, React compared the identical array, bailed out of the
// re-render, and the component kept rendering the stale `loading: true`.
//
// Forever. Every empty list in every Pylon app — a new account's first
// dashboard, a newly added entity, a list whose rows were all deleted — sat on
// its skeleton and never reached its empty state. It was invisible to the
// existing tests because the engine-level signal (packages/sync's
// initial-sync-loading.test.ts) was correct the whole time; only the mounted
// hook was wrong, and packages/react had no renderer to mount it with.
//
// These tests mount the real hook against the real SyncEngine. They fail with
// the ref-based implementation and pass with the derived one.

import { afterEach, describe, expect, test } from "bun:test";
import { act, render, screen, waitFor, cleanup } from "@testing-library/react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useQuery, useQueryOne } from "./hooks";

// @pylonsync/sync points `main` at src/ but `types` at dist/, so the harness
// (imported by its src path) hands back a SyncEngine whose declaration differs
// from the one the hooks are typed against — same class at runtime, two
// nominal types to tsc. One cast here beats reshaping the package's exports.
const asEngine = (e: TestEnv["engine"]): SyncEngine => e as unknown as SyncEngine;

let env: TestEnv | null = null;

afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

function ListProbe({ engine }: { engine: SyncEngine }) {
  const { data, loading } = useQuery<{ id: string }>(engine, "Todo");
  // Render the two facts a caller branches on. A component that never
  // re-renders keeps reporting the values from its last render, which is
  // exactly the failure being pinned.
  return (
    <div>
      <span data-testid="state">{loading ? "loading" : "settled"}</span>
      <span data-testid="count">{data.length}</span>
    </div>
  );
}

function RowProbe({ engine, id }: { engine: SyncEngine; id: string }) {
  const { data, loading } = useQueryOne<{ id: string }>(engine, "Todo", id);
  return (
    <span data-testid="state">
      {loading ? "loading" : data === null ? "not-found" : "found"}
    </span>
  );
}

describe("useQuery loading over an EMPTY entity", () => {
  test("leaves loading once the initial pull settles, so the empty state renders", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });

    render(<ListProbe engine={asEngine(env.engine)} />);

    // Before the pull confirms, an empty local replica is "we don't know yet",
    // not "there is nothing" — callers must be able to hold a skeleton here
    // rather than flash an empty state over rows still in flight.
    expect(screen.getByTestId("state").textContent).toBe("loading");

    await act(async () => {
      await env!.start();
    });

    // The crux. The engine settles with ZERO Todo rows, so the row snapshot is
    // identical across the settle. If `loading` is a ref flipped inside
    // getSnapshot, React bails out of this re-render and the probe is stuck
    // reading "loading" — this waitFor times out.
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("settled"),
    );
    expect(screen.getByTestId("count").textContent).toBe("0");
  });

  test("a replica wipe returns an empty list to loading rather than flashing empty", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    render(<ListProbe engine={asEngine(env.engine)} />);
    await act(async () => {
      await env!.start();
    });
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("settled"),
    );

    // The org-switch path wipes the replica. The rows are empty on both sides
    // of that, so a latched `loading` would stay false and the UI would assert
    // "nothing here" about an org it has not fetched yet.
    await act(async () => {
      await env!.engine.resetReplica();
    });
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("loading"),
    );

    await act(async () => {
      await env!.flush();
      await env!.engine.pull();
    });
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("settled"),
    );
  });
});

describe("useQueryOne loading over a MISSING row", () => {
  test("reaches not-found instead of pinning on loading", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });

    render(<RowProbe engine={asEngine(env.engine)} id="does-not-exist" />);
    expect(screen.getByTestId("state").textContent).toBe("loading");

    await act(async () => {
      await env!.start();
    });

    // Same identical-snapshot trap, and it bit harder here: a row that does
    // not exist yields `null` before AND after the pull, so nothing ever
    // re-rendered and "not found" was unreachable.
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toBe("not-found"),
    );
  });
});
