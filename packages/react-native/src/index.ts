// ---------------------------------------------------------------------------
// @statecraft/react-native
//
// React Native adapter for statecraft. Provides the same hook-based API as
// @statecraft/react, plus RN-specific utilities for offline storage and
// network status monitoring.
// ---------------------------------------------------------------------------

// Core hooks (same API as web — RN has React + fetch, so they work as-is)
export {
  useQuery,
  useQueryOne,
  useMutation,
  useAction,
  useLiveList,
  useLiveRow,
  useInsert,
  useUpdate,
  useDelete,
} from "./hooks";

// Room hook
export { useRoom } from "./useRoom";
export type {
  RoomPeer,
  RoomSnapshot,
  UseRoomOptions,
  UseRoomReturn,
} from "./useRoom";

// One-liner API
export { db, init } from "./db";

// React Native specific — offline persistence
export { AsyncStoragePersistence, OfflineStore } from "./storage";
export type { PersistenceAdapter } from "./storage";

// React Native specific — network status
export { useNetworkStatus } from "./useNetworkStatus";
export type { NetworkStatus } from "./useNetworkStatus";

// Re-export from SDK
export { defineRoute } from "@statecraft/sdk";
export type { RouteMode, AppManifest } from "@statecraft/sdk";

// Re-export sync engine for direct use
export {
  SyncEngine,
  createSyncEngine,
  getServerData,
  LocalStore,
  MutationQueue,
} from "@statecraft/sync";
export type {
  ChangeEvent,
  SyncCursor,
  PullResponse,
  HydrationData,
  Row,
} from "@statecraft/sync";

// ---------------------------------------------------------------------------
// Client context (direct API access without sync engine)
// ---------------------------------------------------------------------------

export interface AgentDBClientConfig {
  baseUrl?: string;
}

let _baseUrl = "http://localhost:4321";

export function configureClient(config: AgentDBClientConfig): void {
  if (config.baseUrl) {
    _baseUrl = config.baseUrl;
  }
}

async function apiRequest(
  method: string,
  path: string,
  body?: unknown,
): Promise<unknown> {
  const res = await fetch(`${_baseUrl}${path}`, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    const errorObj = err?.error as Record<string, unknown> | undefined;
    throw new Error((errorObj?.message as string) ?? `HTTP ${res.status}`);
  }
  return res.json();
}

// ---------------------------------------------------------------------------
// Direct data access (non-synced, for one-shot reads / writes)
// ---------------------------------------------------------------------------

export async function fetchList(
  entity: string,
): Promise<Record<string, unknown>[]> {
  return apiRequest("GET", `/api/entities/${entity}`) as Promise<
    Record<string, unknown>[]
  >;
}

export async function fetchById(
  entity: string,
  id: string,
): Promise<Record<string, unknown> | null> {
  try {
    return (await apiRequest(
      "GET",
      `/api/entities/${entity}/${id}`,
    )) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export async function insert(
  entity: string,
  data: Record<string, unknown>,
): Promise<{ id: string }> {
  return apiRequest("POST", `/api/entities/${entity}`, data) as Promise<{
    id: string;
  }>;
}

export async function update(
  entity: string,
  id: string,
  data: Record<string, unknown>,
): Promise<{ updated: boolean }> {
  return apiRequest(
    "PATCH",
    `/api/entities/${entity}/${id}`,
    data,
  ) as Promise<{ updated: boolean }>;
}

export async function remove(
  entity: string,
  id: string,
): Promise<{ deleted: boolean }> {
  return apiRequest("DELETE", `/api/entities/${entity}/${id}`) as Promise<{
    deleted: boolean;
  }>;
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

export async function createSession(
  userId: string,
): Promise<{ token: string; user_id: string }> {
  return apiRequest("POST", "/api/auth/session", {
    user_id: userId,
  }) as Promise<{ token: string; user_id: string }>;
}

export async function getAuthContext(
  token?: string,
): Promise<{ user_id: string | null }> {
  const headers: Record<string, string> = {};
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  const res = await fetch(`${_baseUrl}/api/auth/me`, { headers });
  return res.json() as Promise<{ user_id: string | null }>;
}
