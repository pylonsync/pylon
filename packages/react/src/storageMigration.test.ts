// Regression: a fresh app defaulted `_appName` to "default", so `storageKey`
// returned the unprefixed `pylon_token`. Two apps on one origin (every app on
// localhost:4321 in dev) then shared one token keyspace and clobbered each
// other's sessions. The fix auto-namespaces per app; this pins that adopting a
// namespace copies an existing default-keyspace session up (non-destructively),
// so no one gets logged out on the switch.
//
// `_appName` is module-global and only transitions off "default" once, so the
// no-clobber branch lives in its own file (storageMigrationNoClobber.test.ts)
// with a fresh transition.

import { expect, test } from "bun:test";

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
