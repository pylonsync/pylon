/**
 * Auth client for the auction house demo. Wraps Pylon's password +
 * guest endpoints and exposes a `useAuth()` hook backed by
 * localStorage + a synthetic event so every component re-renders on
 * sign in / out.
 */
import { useCallback, useEffect, useState } from "react";
import { configureClient, db, storageKey } from "@pylonsync/react";
import type { AuthUser } from "./types";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
const AUTH_EVENT = "pylon-auth-changed";

type StoredAuth = {
  token: string | null;
  userId: string | null;
  isGuest: boolean;
};

function readStored(): StoredAuth {
  if (typeof window === "undefined") {
    return { token: null, userId: null, isGuest: false };
  }
  return {
    token: window.localStorage.getItem(storageKey("token")),
    userId: window.localStorage.getItem(storageKey("userId")),
    isGuest: window.localStorage.getItem(storageKey("isGuest")) === "1",
  };
}

function writeStored({ token, userId, isGuest }: StoredAuth) {
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

export async function ensureGuestSession(): Promise<StoredAuth> {
  const existing = readStored();
  if (existing.token) return existing;
  try {
    const body = await postJson<{ token: string; user_id: string }>(
      "/api/auth/guest",
      {},
    );
    const next = { token: body.token, userId: body.user_id, isGuest: true };
    writeStored(next);
    configureClient({ baseUrl: BASE_URL, appName: "auction-house" });
    return next;
  } catch {
    return existing;
  }
}

export async function register(input: {
  email: string;
  password: string;
  displayName?: string;
}): Promise<StoredAuth> {
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
  configureClient({ baseUrl: BASE_URL, appName: "auction-house" });
  return next;
}

export async function login(input: {
  email: string;
  password: string;
}): Promise<StoredAuth> {
  const body = await postJson<{ token: string; user_id: string }>(
    "/api/auth/password/login",
    input,
  );
  const next = { token: body.token, userId: body.user_id, isGuest: false };
  writeStored(next);
  configureClient({ baseUrl: BASE_URL, appName: "auction-house" });
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

export function useAuth(): {
  user: AuthUser;
  loading: boolean;
  isGuest: boolean;
  isAuthenticated: boolean;
  refresh: () => void;
} {
  const [stored, setStored] = useState<StoredAuth>(() => readStored());

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
    balanceCents?: number;
  }>("User", stored.isGuest || !stored.userId ? "" : stored.userId);

  const refresh = useCallback(() => setStored(readStored()), []);

  if (!stored.userId) {
    return {
      user: null,
      loading: false,
      isGuest: false,
      isAuthenticated: false,
      refresh,
    };
  }
  if (stored.isGuest) {
    return {
      user: { id: stored.userId, displayName: "Guest" },
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
          displayName: userRow.data.displayName,
          balanceCents: userRow.data.balanceCents ?? 0,
        }
      : { id: stored.userId },
    loading: userRow.loading,
    isGuest: false,
    isAuthenticated: true,
    refresh,
  };
}
