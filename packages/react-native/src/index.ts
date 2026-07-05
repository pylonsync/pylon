// ---------------------------------------------------------------------------
// @pylonsync/react-native
//
// Thin layer on top of @pylonsync/react. The hooks, db, useRoom, useSession
// are all the React versions — RN has React + fetch + WebSocket and shares
// the rendering layer, so the entire React API works as-is. This package
// adds:
//
//   - `init()` that bootstraps an AsyncStorage-backed Storage adapter so
//     the sync engine and React free helpers persist tokens / client_id
//     across cold launches
//   - `useNetworkStatus` — RN-specific NetInfo subscription
//   - `AsyncStoragePersistence` / `OfflineStore` — optional manual cache
//     for apps that want a separate offline layer
//
// Why no parallel hook implementations? Re-exporting from React keeps the
// two surfaces in lockstep.
// ---------------------------------------------------------------------------

import {
  init as reactInit,
  setReactStorage,
} from "@pylonsync/react";
import type { SyncEngineConfig } from "@pylonsync/sync";
import { createAsyncStorageBridge } from "./storage";
import { AsyncStorageReplicaPersistence } from "./replica-persistence";

// All hooks, db, callFn, configureClient, etc.
export * from "@pylonsync/react";

// React Native specific.
export { useNetworkStatus } from "./useNetworkStatus";
export type { NetworkStatus } from "./useNetworkStatus";
export { AsyncStorageReplicaPersistence } from "./replica-persistence";
export {
  AsyncStoragePersistence,
  OfflineStore,
  createAsyncStorageBridge,
  type PersistenceAdapter,
} from "./storage";

/**
 * Initialize Pylon for a React Native app. Reads the persisted token /
 * client_id from AsyncStorage, registers the bridge with both the sync
 * engine and the React free helpers, then starts sync.
 *
 * Returns a Promise so RN apps can `await init(...)` before rendering —
 * otherwise the first paint would render against an unauthenticated cache.
 *
 * ```tsx
 * await init({ baseUrl: "https://api.example.com" });
 * AppRegistry.registerComponent("App", () => RootApp);
 * ```
 */
export async function init(
  config?: Partial<SyncEngineConfig> & { baseUrl?: string },
): Promise<void> {
  const storage = await createAsyncStorageBridge();
  setReactStorage(storage);
  // Durable replica by default — without it a cold offline launch renders
  // an empty store (RN has no IndexedDB for the engine to fall back to).
  // Callers can still pass their own adapter or `persist: false`.
  const persistence =
    config?.persistence ??
    (config?.persist === false
      ? undefined
      : new AsyncStorageReplicaPersistence(config?.appName ?? "default"));
  reactInit({ ...config, storage, persistence });
}
