// The legacy-token adoption (see storageMigration.test.ts) must never overwrite
// a token already living in the app namespace — that would replay a stale
// default-keyspace session over the app's real one. Own file so `_appName`
// starts at "default" and the single transition runs the guard.

import { expect, test } from "bun:test";

import { configureClient, setReactStorage } from "./index";
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

test("does not clobber a token already in the app namespace", () => {
  const store = memStorage();
  setReactStorage(store);
  store.set("pylon_token", "tok_legacy");
  store.set("pylon:revtrail:token", "tok_current");

  configureClient({ appName: "revtrail" });

  expect(store.get("pylon:revtrail:token")).toBe("tok_current");
});
