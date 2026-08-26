// A fresh app defaulted `_appName` to "default", so `storageKey("token")`
// returned the unprefixed `pylon_token`. Two apps on one origin (every app on
// localhost:4321 in dev) then shared one token keyspace and clobbered each
// other's sessions. When an app adopts a namespace, `configureClient` copies a
// legacy default-keyspace session up so a single-app upgrade doesn't log anyone
// out — but ONLY when the legacy key is plausibly ours (no other app has
// namespaced here). These pin adopt / no-clobber / don't-import-a-stranger.
//
// `_appName` is a module global that only transitions off "default" once per
// process, so `beforeEach` resets it (configureClient({ appName: "default" }))
// and clears the origin's localStorage that the shared-origin guard reads.

import { beforeEach, expect, test } from "bun:test";

import { configureClient, setReactStorage, storageKey } from "./index";
import type { Storage as PylonStorage } from "@pylonsync/sync";

function memStorage(): PylonStorage & { map: Map<string, string> } {
  const map = new Map<string, string>();
  return {
    map,
    get: (k: string) => map.get(k) ?? null,
    set: (k: string, v: string) => void map.set(k, v),
    remove: (k: string) => void map.delete(k),
  };
}

beforeEach(() => {
  configureClient({ appName: "default" }); // reset the module-global _appName
  if (typeof localStorage !== "undefined") localStorage.clear();
});

test("adopts a legacy pylon_token into the app namespace, non-destructively", () => {
  const store = memStorage();
  setReactStorage(store);
  store.set("pylon_token", "tok_legacy");
  store.set("pylon_userId", "user_1");

  configureClient({ appName: "revtrail" });

  // Session preserved under the new namespaced keys…
  expect(store.get("pylon:revtrail:token")).toBe("tok_legacy");
  expect(store.get("pylon:revtrail:userId")).toBe("user_1");
  // …and COPIED, not moved — a framework rollback still finds the old key.
  expect(store.get("pylon_token")).toBe("tok_legacy");
  // storageKey now resolves to the namespace.
  expect(storageKey("token")).toBe("pylon:revtrail:token");
});

test("does not clobber a token already in the app namespace", () => {
  const store = memStorage();
  setReactStorage(store);
  store.set("pylon_token", "tok_legacy");
  store.set("pylon:revtrail:token", "tok_current");

  configureClient({ appName: "revtrail" });

  expect(store.get("pylon:revtrail:token")).toBe("tok_current");
});

test("does not adopt a foreign pylon_token when another app is namespaced here", () => {
  // Another Pylon app has already run on this origin (shared-origin repro from
  // a 0.5.2 field report). The ownerless legacy key can't be assumed ours.
  localStorage.setItem("pylon:otherapp:token", "tok_other_app");

  const store = memStorage();
  setReactStorage(store);
  store.set("pylon_token", "tok_FOREIGN_from_another_app");

  configureClient({ appName: "mastagents" });

  // The stranger's token is NOT copied into our namespace…
  expect(store.get("pylon:mastagents:token")).toBeNull();
  // …and the legacy key is untouched (we never delete it either).
  expect(store.get("pylon_token")).toBe("tok_FOREIGN_from_another_app");
});
