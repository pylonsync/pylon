// Sendblue adapter (https://docs.sendblue.com).
//
// Inbound: Sendblue POSTs a JSON body to the receive webhook. Authenticity:
// when a webhook secret is configured in Sendblue, every delivery carries it
// verbatim in the `sb-signing-secret` header. That header is compared in
// constant time against SENDBLUE_WEBHOOK_SECRET; a missing or wrong value is
// rejected. The secret is not bound to the body, so `message_handle`
// idempotency (Sendblue retries on 5xx) also absorbs a replayed delivery.
//
// Outbound: POST https://api.sendblue.co/api/send-message with the
// `sb-api-key-id` / `sb-api-secret-key` headers.

import { normalizeHandle } from "./handles";
import { checkSecret, headerValue, type SecretCheck } from "./secrets";
import type {
  InboundMessage,
  MessageTransport,
  OutboundMessage,
  SendResult,
} from "./transport";

export const SENDBLUE_SEND_URL = "https://api.sendblue.co/api/send-message";
export const SENDBLUE_SECRET_HEADER = "sb-signing-secret";
/** Sendblue rejects longer messages. */
export const SENDBLUE_MAX_CHARS = 18_000;

export interface SendblueConfig {
  apiKey: string;
  apiSecret: string;
  fromNumber: string;
  webhookSecret: string;
  dryRun: boolean;
}

type Env = Record<string, string | undefined>;

export function sendblueConfig(env: Env, dryRun: boolean): SendblueConfig {
  return {
    apiKey: (env.SENDBLUE_API_KEY ?? "").trim(),
    apiSecret: (env.SENDBLUE_API_SECRET ?? "").trim(),
    fromNumber: normalizeHandle(env.SENDBLUE_FROM_NUMBER ?? "")?.handle ?? "",
    webhookSecret: (env.SENDBLUE_WEBHOOK_SECRET ?? "").trim(),
    dryRun,
  };
}

/** Verify a webhook delivery's `sb-signing-secret` header. */
export function verifySendblueWebhook(
  headers: Record<string, string> | null | undefined,
  webhookSecret: string,
): SecretCheck {
  return checkSecret(headerValue(headers, SENDBLUE_SECRET_HEADER), webhookSecret);
}

/** The subset of Sendblue's receive-webhook body this app reads. */
interface SendblueInboundBody {
  message_handle?: unknown;
  content?: unknown;
  from_number?: unknown;
  to_number?: unknown;
  is_outbound?: unknown;
  status?: unknown;
  date_sent?: unknown;
  service?: unknown;
  media_url?: unknown;
  group_id?: unknown;
}

/**
 * Parse a receive-webhook body. Skips outbound echoes and status callbacks,
 * group messages, messages to a number other than ours, and bodies without a
 * message id or sender.
 */
export function parseSendblueInbound(
  payload: unknown,
  ourNumber: string,
): { message: InboundMessage } | { skip: string } {
  if (typeof payload !== "object" || payload === null) return { skip: "body is not an object" };
  const b = payload as SendblueInboundBody;
  if (b.is_outbound === true) return { skip: "outbound status event" };
  if (typeof b.group_id === "string" && b.group_id !== "") return { skip: "group message" };
  const externalId = typeof b.message_handle === "string" ? b.message_handle.trim() : "";
  if (externalId === "" || externalId.length > 200) return { skip: "no message_handle" };
  const sender = normalizeHandle(typeof b.from_number === "string" ? b.from_number : "");
  if (!sender) return { skip: "unrecognized sender" };
  if (ourNumber !== "" && typeof b.to_number === "string" && b.to_number.trim() !== "") {
    const to = normalizeHandle(b.to_number);
    if (to && to.handle !== ourNumber) return { skip: "sent to a different number" };
  }
  let text = typeof b.content === "string" ? b.content : "";
  if (text.trim() === "" && typeof b.media_url === "string" && b.media_url !== "") {
    text = "[sent an attachment]";
  }
  if (text.trim() === "") return { skip: "empty message" };
  const sentAtMs = typeof b.date_sent === "string" ? Date.parse(b.date_sent) : Number.NaN;
  return {
    message: {
      externalId: `sendblue:${externalId}`,
      handle: sender.handle,
      text,
      sentAt: new Date(Number.isFinite(sentAtMs) ? sentAtMs : Date.now()).toISOString(),
      service: typeof b.service === "string" && b.service !== "" ? b.service : "iMessage",
    },
  };
}

/** The HTTP request that sends one message. Separate from `fetch` so tests can inspect it. */
export function buildSendblueRequest(
  config: SendblueConfig,
  message: OutboundMessage,
): { url: string; init: RequestInit } {
  return {
    url: SENDBLUE_SEND_URL,
    init: {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "sb-api-key-id": config.apiKey,
        "sb-api-secret-key": config.apiSecret,
      },
      body: JSON.stringify({
        number: message.to,
        from_number: config.fromNumber,
        content: message.text.slice(0, SENDBLUE_MAX_CHARS),
      }),
    },
  };
}

export type FetchLike = (url: string, init: RequestInit) => Promise<Response>;

export function sendblueTransport(
  config: SendblueConfig,
  fetchImpl: FetchLike = fetch,
): MessageTransport {
  return {
    name: "sendblue",
    normalizeInbound: (payload) => parseSendblueInbound(payload, config.fromNumber),
    async send(message): Promise<SendResult> {
      if (config.dryRun) return { status: "dry_run" };
      if (!config.apiKey || !config.apiSecret || !config.fromNumber) {
        return { status: "failed", error: "Sendblue is not configured" };
      }
      const { url, init } = buildSendblueRequest(config, message);
      let res: Response;
      try {
        res = await fetchImpl(url, { ...init, signal: AbortSignal.timeout(20_000) });
      } catch (err) {
        return { status: "failed", error: `Sendblue unreachable: ${errorText(err)}` };
      }
      let body: Record<string, unknown> = {};
      try {
        body = (await res.json()) as Record<string, unknown>;
      } catch {
        // A non-JSON body is reported through the status code below.
      }
      const status = typeof body.status === "string" ? body.status.toUpperCase() : "";
      if (!res.ok || status === "ERROR") {
        const detail =
          (typeof body.error_message === "string" && body.error_message) ||
          (typeof body.message === "string" && body.message) ||
          `HTTP ${res.status}`;
        return { status: "failed", error: `Sendblue: ${detail}`.slice(0, 300) };
      }
      const handle = typeof body.message_handle === "string" ? body.message_handle : null;
      return { status: "sent", externalId: handle ? `sendblue:${handle}` : null };
    },
  };
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
