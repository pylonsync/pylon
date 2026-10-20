// The SSR client runtime calls registerOfflineWorker on every page load. It
// must register `/pylon-sw.js` only for a production page of an app that
// opted in, and otherwise remove a Pylon worker an earlier build left behind
// without touching the app's own workers.

import { expect, test } from "bun:test";
import { registerOfflineWorker } from "./offlineWorker";

function fakeContainer(scripts: string[] = []) {
  const registered: Array<{ url: string; scope?: string }> = [];
  const unregistered: string[] = [];
  const messages: unknown[] = [];
  const container = {
    register: async (url: string, options?: { scope?: string }) => {
      registered.push({ url, scope: options?.scope });
      return { scope: "/" };
    },
    getRegistrations: async () =>
      scripts.map((scriptURL) => ({
        active: { scriptURL },
        unregister: async () => {
          unregistered.push(scriptURL);
          return true;
        },
      })),
    ready: Promise.resolve({ active: { postMessage: (m: unknown) => messages.push(m) } }),
  };
  return { container, registered, unregistered, messages };
}

const loaded = () => Promise.resolve();

test("registers the worker in production when the app opted in", async () => {
  const f = fakeContainer();
  const reg = await registerOfflineWorker({
    optedIn: true,
    production: true,
    signedIn: true,
    navigator: { serviceWorker: f.container },
    whenLoaded: loaded,
  });
  expect(reg).not.toBeNull();
  expect(f.registered).toEqual([{ url: "/pylon-sw.js", scope: "/" }]);
  expect(f.messages).toEqual([{ type: "pylon:session", signedIn: true }]);
});

test("does not register under pylon dev", async () => {
  const f = fakeContainer();
  await registerOfflineWorker({
    optedIn: true,
    production: false,
    navigator: { serviceWorker: f.container },
    whenLoaded: loaded,
  });
  expect(f.registered).toEqual([]);
});

test("does not register when the app did not opt in", async () => {
  const f = fakeContainer();
  await registerOfflineWorker({
    optedIn: false,
    production: true,
    navigator: { serviceWorker: f.container },
    whenLoaded: loaded,
  });
  expect(f.registered).toEqual([]);
});

test("removes a leftover Pylon worker but not the app's own", async () => {
  const f = fakeContainer([
    "https://app.example/pylon-sw.js",
    "https://app.example/push-worker.js",
  ]);
  await registerOfflineWorker({
    optedIn: false,
    production: true,
    navigator: { serviceWorker: f.container },
    whenLoaded: loaded,
  });
  expect(f.unregistered).toEqual(["https://app.example/pylon-sw.js"]);
});

test("a browser without service workers is a no-op", async () => {
  expect(
    await registerOfflineWorker({ optedIn: true, production: true, navigator: {} }),
  ).toBeNull();
});
