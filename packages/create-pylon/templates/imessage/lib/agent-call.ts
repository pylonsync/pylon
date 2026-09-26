// Run the `assistant` agent for an inbound text.
//
// `agent()` persists every turn into AgentRun/AgentMessage rows owned by the
// caller, so it needs a signed-in user. Inbound texts arrive on a webhook or a
// relay sync, where there is no user. The server therefore calls the agent
// over HTTP as the owner: it mints a session for the owner's user id with
// POST /api/auth/session (open in `pylon dev`; requires PYLON_ADMIN_TOKEN in
// production), then POSTs /api/fn/assistant with that session. The owner's
// session resolves to an admin (PYLON_ADMIN_EMAILS), which is the only caller
// the agent accepts. The runs then show up live in the owner's dashboard
// through the framework's owner-scoped AgentRun/AgentMessage sync.
//
// The session token lives only in this process's memory and is reused until
// the server rejects it.

import { AGENT_NAME } from "./assistant";

type Env = Record<string, string | undefined>;

export interface AgentResult {
  runId: string;
  text: string;
  steps: number;
  queued?: boolean;
  cancelled?: boolean;
}

export type AgentCallOutcome =
  | { ok: true; result: AgentResult }
  | { ok: false; code: string; message: string };

export type FetchLike = (url: string, init: RequestInit) => Promise<Response>;

/**
 * Origin the server calls itself on. The request carries PYLON_ADMIN_TOKEN, so
 * it stays on this machine whenever the port is known (PYLON_PORT). Otherwise
 * it goes to APP_URL / PYLON_PUBLIC_URL, and only over https: a plain-http
 * public origin would put the admin token on the wire. Falls back to the
 * default local port.
 */
export function selfBaseUrl(env: Env): string {
  const port = Number.parseInt(env.PYLON_PORT ?? "", 10);
  if (Number.isFinite(port) && port > 0 && port < 65536) return `http://127.0.0.1:${port}`;
  const raw = (env.APP_URL || env.PYLON_PUBLIC_URL || "").trim();
  if (raw !== "") {
    try {
      const u = new URL(raw);
      if (u.protocol === "https:") return u.origin;
    } catch {
      // Fall through to the local default.
    }
  }
  return "http://127.0.0.1:4321";
}

export interface SseEvent {
  event: string;
  data: string;
}

/** Parse a complete text/event-stream body into events. Comment and id lines are dropped. */
export function parseSse(body: string): SseEvent[] {
  const events: SseEvent[] = [];
  for (const block of body.replace(/\r\n?/g, "\n").split("\n\n")) {
    let event = "message";
    const data: string[] = [];
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
    }
    if (data.length > 0) events.push({ event, data: data.join("\n") });
  }
  return events;
}

/** True once `body` holds a complete `result` or `error` event. */
export function hasTerminalEvent(body: string): boolean {
  const complete = body.replace(/\r\n?/g, "\n");
  const end = complete.lastIndexOf("\n\n");
  if (end === -1) return false;
  return parseSse(complete.slice(0, end)).some((e) => e.event === "result" || e.event === "error");
}

/**
 * Read an SSE response until its `result` or `error` event, then stop. The
 * server can hold a finished stream open (it stays resumable for a while), so
 * waiting for the connection to close would hang.
 */
async function readUntilTerminalEvent(res: Response): Promise<string> {
  if (!res.body) return "";
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let body = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      body += decoder.decode(value, { stream: true });
      if (hasTerminalEvent(body)) break;
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  return body;
}

function asResult(value: unknown): AgentResult | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  if (typeof v.runId !== "string") return null;
  return {
    runId: v.runId,
    text: typeof v.text === "string" ? v.text : "",
    steps: typeof v.steps === "number" ? v.steps : 0,
    queued: v.queued === true,
    cancelled: v.cancelled === true,
  };
}

function asError(value: unknown, fallback: string): { code: string; message: string } {
  const v = (value ?? {}) as Record<string, unknown>;
  const e = (typeof v.error === "object" && v.error !== null ? v.error : v) as Record<string, unknown>;
  return {
    code: typeof e.code === "string" ? e.code : "AGENT_FAILED",
    message: typeof e.message === "string" ? e.message : fallback,
  };
}

/** Interpret the agent endpoint's response: SSE when it streamed, JSON otherwise. */
export function readAgentResponse(status: number, contentType: string, body: string): AgentCallOutcome {
  if (contentType.includes("text/event-stream")) {
    const events = parseSse(body);
    const result = events.find((e) => e.event === "result");
    if (result) {
      try {
        const parsed = asResult(JSON.parse(result.data));
        if (parsed) return { ok: true, result: parsed };
      } catch {
        // Reported below.
      }
    }
    const error = events.find((e) => e.event === "error");
    if (error) {
      try {
        return { ok: false, ...asError(JSON.parse(error.data), error.data) };
      } catch {
        return { ok: false, code: "AGENT_FAILED", message: error.data.slice(0, 300) };
      }
    }
    return { ok: false, code: "AGENT_NO_RESULT", message: "The agent stream ended without a result" };
  }
  let json: unknown = null;
  try {
    json = JSON.parse(body);
  } catch {
    return { ok: false, code: `HTTP_${status}`, message: body.slice(0, 300) || `HTTP ${status}` };
  }
  if (status >= 200 && status < 300) {
    const parsed = asResult(json);
    if (parsed) return { ok: true, result: parsed };
  }
  return { ok: false, ...asError(json, `HTTP ${status}`) };
}

interface CachedSession {
  userId: string;
  token: string;
}

let cached: CachedSession | null = null;

async function mintSession(
  base: string,
  userId: string,
  adminToken: string,
  fetchImpl: FetchLike,
): Promise<string> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (adminToken) headers.authorization = `Bearer ${adminToken}`;
  const res = await fetchImpl(`${base}/api/auth/session`, {
    method: "POST",
    headers,
    body: JSON.stringify({ user_id: userId }),
    signal: AbortSignal.timeout(15_000),
  });
  const body = (await res.json().catch(() => ({}))) as Record<string, unknown>;
  if (!res.ok || typeof body.token !== "string") {
    const err = asError(body, `HTTP ${res.status}`);
    throw Object.assign(new Error(err.message), {
      code: res.status === 403 ? "OWNER_SESSION_FORBIDDEN" : err.code,
    });
  }
  return body.token;
}

export interface AgentCallArgs {
  input: string;
  runId?: string;
  title?: string;
}

export async function runAgentAsOwner(
  env: Env,
  ownerUserId: string,
  args: AgentCallArgs,
  fetchImpl: FetchLike = fetch,
): Promise<AgentCallOutcome> {
  const base = selfBaseUrl(env);
  const adminToken = (env.PYLON_ADMIN_TOKEN ?? "").trim();

  for (let attempt = 0; attempt < 2; attempt++) {
    if (!cached || cached.userId !== ownerUserId) {
      try {
        cached = { userId: ownerUserId, token: await mintSession(base, ownerUserId, adminToken, fetchImpl) };
      } catch (err) {
        const e = err as { code?: string; message?: string };
        return { ok: false, code: e.code ?? "OWNER_SESSION_FAILED", message: e.message ?? String(err) };
      }
    }
    let res: Response;
    try {
      res = await fetchImpl(`${base}/api/fn/${AGENT_NAME}`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          accept: "text/event-stream",
          authorization: `Bearer ${cached.token}`,
        },
        body: JSON.stringify(args),
        signal: AbortSignal.timeout(280_000),
      });
    } catch (err) {
      return { ok: false, code: "AGENT_UNREACHABLE", message: err instanceof Error ? err.message : String(err) };
    }
    const contentType = res.headers.get("content-type") ?? "";
    const body = contentType.includes("text/event-stream") ? await readUntilTerminalEvent(res) : await res.text();
    const outcome = readAgentResponse(res.status, contentType, body);
    // A session the server no longer accepts: mint a new one and retry once.
    if (!outcome.ok && (res.status === 401 || res.status === 403) && attempt === 0) {
      cached = null;
      continue;
    }
    return outcome;
  }
  return { ok: false, code: "AGENT_AUTH_FAILED", message: "Could not authenticate as the owner" };
}

/** Test seam: forget the cached owner session. */
export function resetOwnerSessionCache(): void {
  cached = null;
}
