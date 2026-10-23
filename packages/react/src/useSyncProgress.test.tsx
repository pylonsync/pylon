// useSyncProgress re-renders as a pull runs and ends.

import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useSyncProgress } from "./useSyncProgress";

let env: TestEnv | null = null;
afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

function Probe({ engine }: { engine: SyncEngine }) {
  const p = useSyncProgress(engine);
  return <span data-testid="p">{`${p.pulling} ${p.snapshot} ${p.rows} ${p.synced}`}</span>;
}

test("reports a snapshot pull's rows, then synced", async () => {
  env = createTestEnv({ transport: "poll" });
  env.server.snapshotMode = true;
  env.server.seed("Lead", Array.from({ length: 1500 }, (_, i) => ({ id: `l${i}` })));
  env.signIn({ userId: "u1" });
  const engine = env.engine as unknown as SyncEngine;
  render(<Probe engine={engine} />);
  expect(screen.getByTestId("p").textContent).toBe("false false 0 false");

  await act(async () => {
    await env!.engine.start();
  });
  await waitFor(() => expect(screen.getByTestId("p").textContent).toBe("false true 1500 true"));
});
