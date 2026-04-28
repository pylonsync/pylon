// Pylon API base URL.
// In dev, Vite's proxy in vite.config.ts forwards /api → :4321.
// In production, set VITE_PYLON_URL to your deployed Pylon URL.
export const PYLON_URL = import.meta.env.VITE_PYLON_URL ?? "";

export const TOKEN_KEY = "pylon_token";

export type Me = {
  user_id: string | null;
  tenant_id: string | null;
  is_admin: boolean;
};

export function getToken(): string | null {
  return typeof window !== "undefined"
    ? window.localStorage.getItem(TOKEN_KEY)
    : null;
}

export function setToken(token: string): void {
  window.localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
  window.localStorage.removeItem(TOKEN_KEY);
}

export async function pylonFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const token = getToken();
  const headers = new Headers(init.headers);
  if (token && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  return fetch(`${PYLON_URL}${path}`, { ...init, headers });
}

export async function pylonJson<T = unknown>(path: string, init?: RequestInit): Promise<T> {
  const res = await pylonFetch(path, init);
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${body || res.statusText}`);
  }
  return res.json() as Promise<T>;
}
