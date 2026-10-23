// useAppUpdate re-renders when a new build appears, and AppUpdateBar shows
// the notice and runs apply().

import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { renderToString } from "react-dom/server";

import { AppUpdateBar } from "./AppUpdateBar";
import { createAppUpdate, type AppUpdateEnv } from "./appUpdate";
import { useAppUpdate } from "./useAppUpdate";

afterEach(cleanup);

function env(latest: { version: string; install?: string }, reloads: { n: number }) {
  const e: AppUpdateEnv = {
    serviceWorker: null,
    fetch: async () => ({ ok: true, json: async () => latest }),
    document: null,
    setInterval: () => 0,
    clearInterval: () => {},
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (h) => clearTimeout(h as any),
    now: () => Date.now(),
    reload: () => {
      reloads.n++;
    },
    sessionStorage: null,
    localStorage: null,
    iosStandalone: false,
    pendingWrites: () => null,
    askVersion: async () => null,
    warn: () => {},
  };
  return e;
}

function Probe({ updater }: { updater: ReturnType<typeof createAppUpdate> }) {
  const u = useAppUpdate(updater);
  return <span data-testid="state">{`${u.version} -> ${u.latestVersion} ready=${u.updateReady}`}</span>;
}

test("re-renders when the server reports a new build", async () => {
  const latest = { version: "v1" };
  const updater = createAppUpdate({
    build: { version: "v1", builtAt: null },
    production: true,
    env: env(latest, { n: 0 }),
  });
  render(<Probe updater={updater} />);
  await waitFor(() =>
    expect(screen.getByTestId("state").textContent).toBe("v1 -> v1 ready=false"),
  );
  latest.version = "v2";
  await act(async () => {
    await updater.check();
  });
  expect(screen.getByTestId("state").textContent).toBe("v1 -> v2 ready=true");
});

test("renders the empty state on the server", () => {
  const updater = createAppUpdate({
    build: { version: "v1", builtAt: null },
    production: true,
    env: env({ version: "v2" }, { n: 0 }),
  });
  expect(renderToString(<Probe updater={updater} />)).toContain("null -&gt; null ready=false");
  expect(renderToString(<AppUpdateBar updater={updater} />)).toBe("");
});

test("AppUpdateBar shows the notice when a build is ready, and the button applies it", async () => {
  const reloads = { n: 0 };
  const updater = createAppUpdate({
    build: { version: "v1", builtAt: null },
    production: true,
    env: env({ version: "v2" }, reloads),
  });
  render(<AppUpdateBar updater={updater} className="bar" />);
  const button = await screen.findByRole("button", { name: "Update" });
  expect(screen.getByRole("status").getAttribute("data-pylon-app-update")).toBe("ready");
  expect(screen.getByRole("status").className).toBe("bar");
  await act(async () => {
    fireEvent.click(button);
  });
  await waitFor(() => expect(reloads.n).toBe(1));
});

test("AppUpdateBar renders nothing without an update", async () => {
  const updater = createAppUpdate({
    build: { version: "v1", builtAt: null },
    production: true,
    env: env({ version: "v1" }, { n: 0 }),
  });
  const { container } = render(<AppUpdateBar updater={updater} />);
  await act(async () => {
    await updater.check();
  });
  expect(container.innerHTML).toBe("");
});
