// HTTP client for the server's relaySync endpoint.

import type { RelaySyncRequest, RelaySyncResponse } from "../lib/relay-protocol";

export type FetchLike = (url: string, init: RequestInit) => Promise<Response>;

export class RelayHttpError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export async function relaySync(
  serverUrl: string,
  token: string,
  body: RelaySyncRequest,
  fetchImpl: FetchLike = fetch,
): Promise<RelaySyncResponse> {
  const url = new URL("/api/fn/relaySync", serverUrl);
  const res = await fetchImpl(url.toString(), {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(20_000),
  });
  const json = (await res.json().catch(() => null)) as
    | (RelaySyncResponse & { error?: { code?: string; message?: string } })
    | null;
  if (!res.ok || !json || json.error) {
    const code = json?.error?.code ?? `HTTP_${res.status}`;
    throw new RelayHttpError(res.status, code, json?.error?.message ?? `relaySync failed with HTTP ${res.status}`);
  }
  return {
    accepted: Array.isArray(json.accepted) ? json.accepted.map(String) : [],
    outbound: Array.isArray(json.outbound) ? json.outbound : [],
  };
}
