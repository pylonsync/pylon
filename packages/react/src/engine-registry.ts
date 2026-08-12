// The active SyncEngine, readable without creating one.
//
// `db.ts` owns the engine's lifecycle, and `getSync()` there deliberately
// creates and starts an engine on first access. That is right for `db.*`
// — a call to `db.useQuery` means the app wants sync — but wrong for the
// free helpers in `index.ts`, which must work in apps that never call
// `init()`. `callFn` reaching for `getSync()` would open a WebSocket and
// run an initial pull as a side effect of a single POST.
//
// Hence a separate module: `db.ts` publishes the engine here, `index.ts`
// peeks at it, and neither imports the other. They already form a cycle
// (`db.ts` imports `callFn` from `index.ts`), so the registry has to live
// outside both.

import type { SyncEngine } from "@pylonsync/sync";

let _engine: SyncEngine | null = null;

/** Publish the engine `init()` built. Called only from `db.ts`. */
export function setActiveEngine(engine: SyncEngine | null): void {
  _engine = engine;
}

/**
 * The engine, if one exists. Never constructs one — a `null` return
 * means "this app isn't running sync", not "not yet initialized".
 */
export function peekActiveEngine(): SyncEngine | null {
  return _engine;
}
