"use server";

import { cookies } from "next/headers";
import { redirect } from "next/navigation";

const PYLON_TARGET = process.env.PYLON_TARGET ?? "http://localhost:4321";
const COOKIE_NAME = "__APP_NAME___session";

// Two-step magic-code flow:
//   1. startMagicCode  → POST /api/auth/magic/send  { email }
//   2. verifyMagicCode → POST /api/auth/magic/verify { email, code }
//      → { token, user_id, expires_at }
//      Set as HttpOnly cookie, then redirect to /dashboard.
//
// Errors thrown here are caught by Next's error boundary and shown
// to the user. The client form below also catches and renders.

export async function startMagicCode(email: string): Promise<{ devCode?: string }> {
  const res = await fetch(`${PYLON_TARGET}/api/auth/magic/send`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email }),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Could not send code: ${body}`);
  }
  const json = (await res.json()) as { sent?: boolean; dev_code?: string };
  // In dev mode the server returns the code so you can sign in without
  // an email provider configured. Surface it to the client.
  return { devCode: json.dev_code };
}

export async function verifyMagicCode(formData: FormData) {
  const email = formData.get("email") as string;
  const code = formData.get("code") as string;
  if (!email || !code) {
    throw new Error("Email and code required");
  }
  const res = await fetch(`${PYLON_TARGET}/api/auth/magic/verify`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, code }),
  });
  if (!res.ok) {
    throw new Error("Invalid or expired code");
  }
  const json = (await res.json()) as {
    token: string;
    user_id: string;
    expires_at: number;
  };

  // Set the session cookie. Match the cookieName in lib/pylon.ts.
  // In production, set Secure: true (HTTPS-only).
  const isProd = process.env.NODE_ENV === "production";
  cookies().set(COOKIE_NAME, json.token, {
    httpOnly: true,
    sameSite: "lax",
    secure: isProd,
    path: "/",
    maxAge: Math.max(0, json.expires_at - Math.floor(Date.now() / 1000)),
  });

  redirect("/dashboard");
}

export async function signOut() {
  const token = cookies().get(COOKIE_NAME)?.value;
  if (token) {
    await fetch(`${PYLON_TARGET}/api/auth/session`, {
      method: "DELETE",
      headers: { Authorization: `Bearer ${token}` },
    }).catch(() => {
      // Best-effort — even if the server call fails, clear the cookie
      // locally so the user is signed out from this device.
    });
  }
  cookies().delete(COOKIE_NAME);
  redirect("/");
}
