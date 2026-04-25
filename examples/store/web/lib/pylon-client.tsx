"use client";

/**
 * Client-side Pylon initializer + auth context.
 *
 * Mounted once at the root via `<PylonProvider>`. Inside, components
 * can use `db.useQuery`, `db.useMutation`, etc. and the `useAuth()`
 * hook for password/guest sessions.
 */
import { createContext, useCallback, useContext, useEffect, useState } from "react";
import {
  init,
  configureClient,
  storageKey,
  db,
} from "@pylonsync/react";

const BASE_URL =
  process.env.NEXT_PUBLIC_PYLON_URL ?? "http://localhost:4321";
const WS_URL = BASE_URL.startsWith("https://")
  ? `${BASE_URL.replace(/^https:/, "wss:").replace(/\/$/, "")}:4322`
  : undefined;

let initialized = false;

function initOnce() {
  if (initialized) return;
  initialized = true;
  init({ baseUrl: BASE_URL, appName: "store", wsUrl: WS_URL });
  configureClient({ baseUrl: BASE_URL, appName: "store" });
}

// ---------------------------------------------------------------------------
// Auth state
// ---------------------------------------------------------------------------

export type AuthUser = {
  id: string;
  email?: string;
  name?: string;
  isGuest?: boolean;
} | null;

type AuthState = {
  token: string | null;
  userId: string | null;
  isGuest: boolean;
};

const AUTH_EVENT = "pylon-auth-changed";

function readStored(): AuthState {
  if (typeof window === "undefined") {
    return { token: null, userId: null, isGuest: false };
  }
  return {
    token: window.localStorage.getItem(storageKey("token")),
    userId: window.localStorage.getItem(storageKey("userId")),
    isGuest: window.localStorage.getItem(storageKey("isGuest")) === "1",
  };
}

function writeStored({ token, userId, isGuest }: AuthState) {
  if (typeof window === "undefined") return;
  if (token) window.localStorage.setItem(storageKey("token"), token);
  else window.localStorage.removeItem(storageKey("token"));
  if (userId) window.localStorage.setItem(storageKey("userId"), userId);
  else window.localStorage.removeItem(storageKey("userId"));
  if (isGuest) window.localStorage.setItem(storageKey("isGuest"), "1");
  else window.localStorage.removeItem(storageKey("isGuest"));
  window.dispatchEvent(new Event(AUTH_EVENT));
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) {
    const code =
      (json as { error?: { code?: string } }).error?.code ?? "REQUEST_FAILED";
    const message =
      (json as { error?: { message?: string } }).error?.message ??
      `Request failed (${res.status})`;
    throw Object.assign(new Error(message), { code });
  }
  return json as T;
}

async function ensureGuestSession(): Promise<AuthState> {
  const existing = readStored();
  if (existing.token) return existing;
  try {
    const body = await postJson<{ token: string; user_id: string }>(
      "/api/auth/guest",
      {},
    );
    const next = { token: body.token, userId: body.user_id, isGuest: true };
    writeStored(next);
    configureClient({ baseUrl: BASE_URL, appName: "store" });
    return next;
  } catch {
    return existing;
  }
}

export async function register(input: {
  email: string;
  password: string;
  displayName?: string;
}): Promise<AuthState> {
  const body = await postJson<{ token: string; user_id: string }>(
    "/api/auth/password/register",
    {
      email: input.email,
      password: input.password,
      displayName: input.displayName ?? input.email.split("@")[0],
    },
  );
  const next = { token: body.token, userId: body.user_id, isGuest: false };
  writeStored(next);
  configureClient({ baseUrl: BASE_URL, appName: "store" });
  return next;
}

export async function login(input: {
  email: string;
  password: string;
}): Promise<AuthState> {
  const body = await postJson<{ token: string; user_id: string }>(
    "/api/auth/password/login",
    input,
  );
  const next = { token: body.token, userId: body.user_id, isGuest: false };
  writeStored(next);
  configureClient({ baseUrl: BASE_URL, appName: "store" });
  return next;
}

export async function logout(): Promise<void> {
  const { token } = readStored();
  try {
    if (token) {
      await fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      });
    }
  } catch {}
  writeStored({ token: null, userId: null, isGuest: false });
  await ensureGuestSession();
}

// ---------------------------------------------------------------------------
// Provider + hooks
// ---------------------------------------------------------------------------

const PylonReadyContext = createContext(false);

export function PylonProvider({ children }: { children: React.ReactNode }) {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    initOnce();
    ensureGuestSession().then(() => setReady(true));
  }, []);

  return (
    <PylonReadyContext.Provider value={ready}>
      {children}
    </PylonReadyContext.Provider>
  );
}

export function useAuth(): {
  user: AuthUser;
  loading: boolean;
  isGuest: boolean;
  isAuthenticated: boolean;
  refresh: () => void;
} {
  const ready = useContext(PylonReadyContext);
  const [stored, setStored] = useState<AuthState>(() => readStored());

  useEffect(() => {
    const onChange = () => setStored(readStored());
    window.addEventListener(AUTH_EVENT, onChange);
    window.addEventListener("storage", onChange);
    return () => {
      window.removeEventListener(AUTH_EVENT, onChange);
      window.removeEventListener("storage", onChange);
    };
  }, []);

  const userRow = db.useQueryOne<{
    id: string;
    email?: string;
    displayName?: string;
  }>("User", stored.isGuest || !stored.userId ? "" : stored.userId);

  const refresh = useCallback(() => setStored(readStored()), []);

  if (!stored.userId) {
    return {
      user: null,
      loading: !ready,
      isGuest: false,
      isAuthenticated: false,
      refresh,
    };
  }
  if (stored.isGuest) {
    return {
      user: { id: stored.userId, isGuest: true },
      loading: false,
      isGuest: true,
      isAuthenticated: false,
      refresh,
    };
  }
  return {
    user: userRow.data
      ? {
          id: userRow.data.id,
          email: userRow.data.email,
          name: userRow.data.displayName,
        }
      : { id: stored.userId },
    loading: userRow.loading,
    isGuest: false,
    isAuthenticated: true,
    refresh,
  };
}
