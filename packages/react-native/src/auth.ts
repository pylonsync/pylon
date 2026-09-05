/**
 * Auth for React Native apps.
 *
 * `@pylonsync/client`'s helpers persist the session through
 * `window.localStorage`, which does not exist on a device. These talk to
 * the same server routes and persist through the AsyncStorage bridge that
 * `init()` registered, which is where the sync engine reads the token.
 *
 * Every function returns after the local session is updated, so the next
 * `useAuth()` / `db.useQuery` sees the new identity.
 */
import { db, getBaseUrl, getReactStorage, storageKey } from "@pylonsync/react";

export interface Session {
  token: string;
  user_id?: string | null;
  expires_at?: string | number | null;
  guest?: boolean;
}

export class AuthError extends Error {
  readonly code: string;
  readonly status: number;
  constructor(code: string, message: string, status: number) {
    super(message);
    this.name = "AuthError";
    this.code = code;
    this.status = status;
  }
}

function currentToken(): string | null {
  return getReactStorage().get(storageKey("token")) ?? null;
}

async function request<T>(
  method: "GET" | "POST" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  const headers: Record<string, string> = { accept: "application/json" };
  if (body !== undefined) headers["content-type"] = "application/json";
  const token = currentToken();
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`${getBaseUrl()}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as T & {
    error?: { code?: string; message?: string };
  };
  if (!res.ok) {
    throw new AuthError(
      data.error?.code ?? `HTTP_${res.status}`,
      data.error?.message ?? "Request failed",
      res.status,
    );
  }
  return data;
}

/**
 * Store a session token and tell the sync engine the identity changed.
 * The engine wipes the previous identity's replica and re-pulls.
 */
export function persistSession(session: Session): void {
  getReactStorage().set(storageKey("token"), session.token);
  void db.sync.notifySessionChanged();
}

/** Forget the local session without calling the server. */
export function clearSession(): void {
  getReactStorage().remove(storageKey("token"));
  void db.sync.notifySessionChanged();
}

/**
 * Start as an anonymous guest so the app is usable before sign-up. A
 * later sign-in through any method below merges the guest's rows into
 * the real account (the server's anonymous merge).
 */
export async function guestSession(): Promise<Session> {
  const session = await request<Session>("POST", "/api/auth/guest", {});
  persistSession(session);
  return session;
}

/** Email a 6-digit code. Rate-limited server-side. */
export function sendEmailCode(email: string): Promise<{ sent: boolean }> {
  return request<{ sent: boolean }>("POST", "/api/auth/magic/send", { email });
}

/** Redeem the emailed code for a session. */
export async function verifyEmailCode(email: string, code: string): Promise<Session> {
  const session = await request<Session>("POST", "/api/auth/magic/verify", {
    email,
    code,
  });
  persistSession(session);
  return session;
}

export async function passwordLogin(email: string, password: string): Promise<Session> {
  const session = await request<Session>("POST", "/api/auth/password/login", {
    email,
    password,
  });
  persistSession(session);
  return session;
}

export async function passwordRegister(
  email: string,
  password: string,
): Promise<Session> {
  const session = await request<Session>("POST", "/api/auth/password/register", {
    email,
    password,
  });
  persistSession(session);
  return session;
}

/**
 * Sign in with an id_token from the platform SDK (Sign in with Apple,
 * Google Sign-In). The server verifies it against the provider's keys
 * (`POST /api/auth/native/<provider>`); the server needs the app's
 * bundle id / client id in PYLON_APPLE_NATIVE_CLIENT_IDS or
 * PYLON_GOOGLE_NATIVE_CLIENT_IDS. `name` is forwarded for Apple, which
 * only reveals it to the app on the first sign-in.
 */
export async function nativeSignIn(
  provider: "apple" | "google",
  idToken: string,
  name?: string | null,
): Promise<Session> {
  const session = await request<Session>("POST", `/api/auth/native/${provider}`, {
    id_token: idToken,
    ...(name ? { name } : {}),
  });
  persistSession(session);
  return session;
}

export interface Me {
  user_id: string | null;
  tenant_id?: string | null;
  is_admin?: boolean;
  roles?: string[];
  guest?: boolean;
}

/** The server's view of the current session, or null when signed out. */
export async function getMe(): Promise<Me | null> {
  if (!currentToken()) return null;
  try {
    return await request<Me>("GET", "/api/auth/me");
  } catch {
    return null;
  }
}

/** Revoke the server session and forget it locally. */
export async function signOut(): Promise<void> {
  if (currentToken()) {
    await request("POST", "/api/auth/logout", {}).catch(() => undefined);
  }
  clearSession();
}

/**
 * Delete the account: the server revokes every session and API key,
 * removes the provider links, and deletes the User row. Required by the
 * App Store for any app that offers account creation.
 */
export async function deleteAccount(): Promise<void> {
  await request("DELETE", "/api/auth/account");
  clearSession();
}
