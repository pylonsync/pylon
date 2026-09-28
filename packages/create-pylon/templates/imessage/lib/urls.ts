type Env = Record<string, string | undefined>;

/**
 * This app's origin, for the URLs the Setup page shows (the Sendblue webhook
 * and the relay endpoint): APP_URL or PYLON_PUBLIC_URL when set to an http(s)
 * URL, otherwise the local dev server.
 */
export function appBaseUrl(env: Env): string {
  const raw = (env.APP_URL || env.PYLON_PUBLIC_URL || "").trim();
  if (raw !== "") {
    try {
      const u = new URL(raw);
      if (u.protocol === "https:" || u.protocol === "http:") return u.origin;
    } catch {
      // Fall through to the local default.
    }
  }
  const port = Number.parseInt(env.PYLON_PORT ?? "", 10);
  return `http://127.0.0.1:${Number.isFinite(port) && port > 0 && port < 65536 ? port : 4321}`;
}
