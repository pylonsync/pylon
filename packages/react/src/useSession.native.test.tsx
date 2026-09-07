// React Native defines `window` as an alias of the global object with no
// `addEventListener`. `useSession` used to subscribe to `storage` events
// whenever `window` existed, which threw inside its mount effect and
// crashed every native app on startup. This mounts the hook under that
// shape of `window` and checks the session still resolves.

import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import type { SyncEngine } from "@pylonsync/sync";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { useSession } from "./useSession";

let env: TestEnv | null = null;

afterEach(async () => {
  cleanup();
  if (env) {
    await env.dispose();
    env = null;
  }
});

function Probe({ engine }: { engine: SyncEngine }) {
  const s = useSession(engine);
  return <span data-testid="s">{s.resolved ? (s.userId ?? "anonymous") : "pending"}</span>;
}

test("mounts where window has no event API and reports resolution", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  const engine = env.engine as unknown as SyncEngine;

  const original = window.addEventListener;
  (window as unknown as { addEventListener: unknown }).addEventListener = undefined;
  try {
    render(<Probe engine={engine} />);
    expect(screen.getByTestId("s").textContent).toBe("pending");
    await act(async () => {
      await engine.start();
    });
    await waitFor(() => expect(screen.getByTestId("s").textContent).toBe("u1"));
  } finally {
    window.addEventListener = original;
  }
});
