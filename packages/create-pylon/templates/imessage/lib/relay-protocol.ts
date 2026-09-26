// The wire contract between the Mac relay (relay/main.ts) and the server
// (functions/relaySync.ts). One endpoint, one round trip per poll:
//
//   relay → server  POST /api/fn/relaySync
//                   Authorization: Bearer <RELAY_TOKEN>
//                   { relayVersion, host, inbound: RelayInboundItem[], acks: RelayAck[] }
//   server → relay  { accepted: string[], outbound: RelayOutboundItem[] }
//
// `accepted` lists the inbound guids the server stored (or had already
// stored); the relay advances its chat.db watermark only past those.
// `outbound` is the next batch of replies to send; each is leased to the relay
// for OUTBOUND_LEASE_MS and returns to the queue when no ack arrives in time.

import { normalizeHandle } from "./handles";
import type { InboundMessage, MessageTransport, SendResult } from "./transport";

export const RELAY_PROTOCOL_VERSION = 1;
export const MAX_INBOUND_PER_SYNC = 100;
export const MAX_OUTBOUND_PER_SYNC = 20;
export const MAX_TEXT_CHARS = 20_000;
export const OUTBOUND_LEASE_MS = 60_000;
/** A relay reply leased this many times without an ack is marked failed. */
export const MAX_RELAY_ATTEMPTS = 5;

export interface RelayInboundItem {
  /** chat.db message.guid — globally unique per message. */
  guid: string;
  handle: string;
  text: string;
  sentAt: string;
  service: string;
}

export interface RelayOutboundItem {
  id: string;
  handle: string;
  text: string;
}

export interface RelayAck {
  id: string;
  ok: boolean;
  error?: string;
  /** The relay runs with RELAY_DRY_RUN and did not hand the message to Messages.app. */
  dryRun?: boolean;
}

export interface RelaySyncRequest {
  relayVersion: string;
  host: string;
  inbound: RelayInboundItem[];
  acks: RelayAck[];
}

export interface RelaySyncResponse {
  accepted: string[];
  outbound: RelayOutboundItem[];
}

const GUID_RE = /^[A-Za-z0-9:._-]{1,128}$/;

export function parseRelayInbound(item: unknown): { message: InboundMessage } | { skip: string } {
  if (typeof item !== "object" || item === null) return { skip: "not an object" };
  const it = item as Partial<RelayInboundItem>;
  if (typeof it.guid !== "string" || !GUID_RE.test(it.guid)) return { skip: "bad guid" };
  const sender = normalizeHandle(typeof it.handle === "string" ? it.handle : "");
  if (!sender) return { skip: "unrecognized sender" };
  const text = typeof it.text === "string" ? it.text.slice(0, MAX_TEXT_CHARS) : "";
  if (text.trim() === "") return { skip: "empty message" };
  const sentAtMs = typeof it.sentAt === "string" ? Date.parse(it.sentAt) : Number.NaN;
  return {
    message: {
      externalId: `relay:${it.guid}`,
      handle: sender.handle,
      text,
      sentAt: new Date(Number.isFinite(sentAtMs) ? sentAtMs : Date.now()).toISOString(),
      service: typeof it.service === "string" && it.service !== "" ? it.service.slice(0, 32) : "iMessage",
    },
  };
}

/**
 * The relay adapter. Sending does not talk to the Mac: the outbound Message
 * row is already stored with status "queued", and the relay collects it on its
 * next sync. `send` therefore only reports that.
 */
export function relayTransport(dryRun: boolean): MessageTransport {
  return {
    name: "relay",
    normalizeInbound: parseRelayInbound,
    async send(): Promise<SendResult> {
      return dryRun ? { status: "dry_run" } : { status: "queued" };
    },
  };
}
