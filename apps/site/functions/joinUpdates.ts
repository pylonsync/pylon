import { mutation, v } from "@pylonsync/functions";

/**
 * Footer "get updates" capture — POST /api/fn/joinUpdates.
 *
 * This app has no database, and the signup list must not fork: the control
 * plane owns `EmailSignup`, the admin views that read it, and the
 * `notifyNewSignups` cron that pings Slack. So this is a RELAY — it validates
 * locally (cheap rejection of junk before it crosses the wire) and forwards the
 * address to the control plane's own public `joinUpdates`, which stays the
 * single writer.
 *
 * Server-side rather than letting the browser POST cross-origin, because a
 * cross-origin call would couple this site to whatever the control plane's
 * CORS + CSRF allowlists happen to be — and those move as the Stack0 Cloud
 * rebrand proceeds. The browser only ever talks to its own origin; the
 * server-to-server hop carries no Origin header, so it is unaffected by either
 * gate. The upstream mutation is `auth: "public"`, so no credential is needed
 * and none is sent.
 */

// Mirrors the control plane's own validation so an obviously-bad address never
// costs a round trip. The upstream check is still the authority.
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/;
const MAX_SOURCE_LEN = 64;
const UPSTREAM_TIMEOUT_MS = 8_000;

export default mutation<
  { email: string; source?: string },
  { ok: boolean; existing?: boolean }
>({
  args: { email: v.string(), source: v.optional(v.string()) },
  auth: "public",
  async handler(ctx, { email, source }) {
    const normalized = email.trim().toLowerCase();
    if (normalized.length > 254 || !EMAIL_RE.test(normalized)) {
      throw new Error("That doesn't look like an email address.");
    }
    const src = (source ?? "footer").slice(0, MAX_SOURCE_LEN);

    // The control plane owns EmailSignup and the Slack notifier, so the
    // signup relays to wherever it lives — the Stack0 Cloud host, not this
    // marketing domain, even though the visitor typed their address here.
    const base = (
      ctx.env.PYLON_CONTROL_PLANE_URL ?? "https://www.usesmallware.com"
    ).replace(/\/$/, "");

    let res: Response;
    try {
      res = await fetch(`${base}/api/fn/joinUpdates`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email: normalized, source: src }),
        // Without a deadline a wedged control plane would hold this request
        // until the function-call ceiling and show the visitor a spinner that
        // never resolves, for a form that is not worth blocking on.
        signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
      });
    } catch {
      throw new Error("That didn't go through — please try again.");
    }

    if (!res.ok) {
      throw new Error("That didn't go through — please try again.");
    }
    const body = (await res.json().catch(() => ({}))) as {
      ok?: boolean;
      existing?: boolean;
    };
    return { ok: body.ok ?? true, existing: body.existing };
  },
});
