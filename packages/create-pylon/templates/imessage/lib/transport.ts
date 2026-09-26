// The one interface both iMessage transports implement. Server code talks to a
// `MessageTransport`; which adapter it gets depends on IMESSAGE_TRANSPORT.
//
//   sendblue  Sendblue's hosted iMessage API. Inbound: signed webhook to
//             /api/webhooks/sendblueWebhook. Outbound: their REST API.
//   relay     A Bun script on an always-on Mac (relay/). Inbound: the relay
//             reads ~/Library/Messages/chat.db and posts new rows to
//             /api/fn/relaySync. Outbound: replies wait in the Message table
//             until the relay's next sync picks them up and sends them with
//             Messages.app.

import { normalizeHandle } from "./handles";

export type TransportName = "sendblue" | "relay";

/** A message a transport received, in the app's shape. */
export interface InboundMessage {
  /** The transport's own message id. Webhooks retry, so this is the dedupe key. */
  externalId: string;
  /** Normalized sender handle (E.164 phone or lowercase email). */
  handle: string;
  text: string;
  /** ISO-8601 time the sender sent it. */
  sentAt: string;
  /** "iMessage" | "SMS" | … as reported by the transport. */
  service: string;
}

/** A message the app wants delivered. */
export interface OutboundMessage {
  /** Id of the outbound Message row this send belongs to. */
  messageId: string;
  to: string;
  text: string;
}

export type SendResult =
  /** The transport accepted the message. */
  | { status: "sent"; externalId: string | null }
  /** The message waits for a pull-based transport (the Mac relay) to collect it. */
  | { status: "queued" }
  /** IMESSAGE_DRY_RUN is on: nothing left the server. */
  | { status: "dry_run" }
  | { status: "failed"; error: string };

export interface MessageTransport {
  readonly name: TransportName;
  /** Turn one raw inbound payload into an InboundMessage, or say why it is skipped. */
  normalizeInbound(payload: unknown): { message: InboundMessage } | { skip: string };
  send(message: OutboundMessage): Promise<SendResult>;
}

type Env = Record<string, string | undefined>;

function present(env: Env, key: string): boolean {
  const v = env[key];
  return typeof v === "string" && v.trim() !== "";
}

/** The transport named by IMESSAGE_TRANSPORT, or null when unset or unknown. */
export function selectedTransport(env: Env): TransportName | null {
  const v = (env.IMESSAGE_TRANSPORT ?? "").trim().toLowerCase();
  return v === "sendblue" || v === "relay" ? v : null;
}

function flag(env: Env, key: string): boolean {
  const v = (env[key] ?? "").trim().toLowerCase();
  return v === "1" || v === "true" || v === "yes";
}

/** `pylon dev` sets PYLON_DEV_MODE=1. */
export function isDevMode(env: Env): boolean {
  return flag(env, "PYLON_DEV_MODE");
}

/**
 * Whether outbound messages are recorded without being sent. True when
 * IMESSAGE_DRY_RUN is set, and always in `pylon dev` unless
 * IMESSAGE_ALLOW_DEV_SENDS=1: a development server is reachable from the local
 * network and hands out sign-in codes in its responses, so it must not be able
 * to text real people by default.
 */
export function isDryRun(env: Env): boolean {
  if (flag(env, "IMESSAGE_DRY_RUN")) return true;
  return isDevMode(env) && !flag(env, "IMESSAGE_ALLOW_DEV_SENDS");
}

export interface SetupItem {
  key: string;
  label: string;
  ok: boolean;
  /** One sentence on what to do when `ok` is false. */
  hint: string;
}

/** What each transport needs, as a checklist. Reports presence only, never values. */
export function transportChecklist(env: Env, name: TransportName): SetupItem[] {
  if (name === "sendblue") {
    const from = normalizeHandle(env.SENDBLUE_FROM_NUMBER ?? "");
    return [
      {
        key: "SENDBLUE_API_KEY",
        label: "Sendblue API key id",
        ok: present(env, "SENDBLUE_API_KEY"),
        hint: "Copy the API key id from the Sendblue dashboard.",
      },
      {
        key: "SENDBLUE_API_SECRET",
        label: "Sendblue API secret",
        ok: present(env, "SENDBLUE_API_SECRET"),
        hint: "Copy the API secret from the Sendblue dashboard.",
      },
      {
        key: "SENDBLUE_FROM_NUMBER",
        label: "Sendblue number",
        ok: from?.kind === "phone",
        hint: "Your Sendblue line in E.164 form, for example +15125550100.",
      },
      {
        key: "SENDBLUE_WEBHOOK_SECRET",
        label: "Webhook secret",
        ok: (env.SENDBLUE_WEBHOOK_SECRET ?? "").trim().length >= 24,
        hint: "At least 24 random characters. Set the same value as the webhook secret in Sendblue.",
      },
    ];
  }
  return [
    {
      key: "RELAY_TOKEN",
      label: "Relay token",
      ok: (env.RELAY_TOKEN ?? "").trim().length >= 24,
      hint: "At least 24 random characters. The relay on your Mac sends the same value.",
    },
  ];
}
