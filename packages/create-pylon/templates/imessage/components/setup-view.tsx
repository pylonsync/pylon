import React, { useState } from "react";
import { Check, Circle, Copy } from "lucide-react";
import { ago } from "@/lib/format";
import { cn } from "@/lib/utils";

export interface SetupStatus {
  transport: "sendblue" | "relay" | null;
  dryRun: boolean;
  checklist: { key: string; label: string; ok: boolean; hint: string }[];
  provider: { configured: boolean; provider: string | null };
  ownerConfigured: boolean;
  appUrlSet: boolean;
  devMode: boolean;
  baseUrl: string;
  webhookUrl: string;
  relayUrl: string;
}

export interface TransportStateView {
  name: string;
  lastInboundAt: string | null;
  lastOutboundAt: string | null;
  lastError: string | null;
  lastErrorAt: string | null;
  relayLastSeenAt: string | null;
  relayVersion: string | null;
  relayHost: string | null;
}

export interface SettingsView {
  ownerName: string;
  timezone: string;
  unknownSenderPolicy: "ignore" | "reply_once";
  unknownSenderReply: string;
  rateLimitPerHour: number;
  paused: boolean;
}

/** A relay that synced in the last 30 seconds is online. */
export function relayOnline(lastSeen: string | null, now = Date.now()): boolean {
  return lastSeen !== null && now - new Date(lastSeen).getTime() < 30_000;
}

/** Setup problems that stop the assistant from answering, for the nav badge. */
export function setupIssueCount(s: SetupStatus | null): number {
  if (!s) return 0;
  let n = 0;
  if (!s.transport) n += 1;
  n += s.checklist.filter((c) => !c.ok).length;
  if (!s.provider.configured) n += 1;
  if (!s.ownerConfigured) n += 1;
  return n;
}

export function SetupView({
  status,
  transport,
  settings,
  hasDemo,
  onSaveSettings,
  onClearDemo,
}: {
  status: SetupStatus | null;
  transport: TransportStateView | null;
  settings: SettingsView;
  hasDemo: boolean;
  onSaveSettings: (patch: Partial<SettingsView>) => Promise<void>;
  onClearDemo: () => Promise<void>;
}) {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-[860px] px-4 py-8 md:px-8 md:py-12">
        <h1 className="text-[28px] font-semibold tracking-[-0.025em]">Setup</h1>
        <p className="mt-1.5 max-w-[60ch] text-[14px] leading-relaxed text-muted-foreground">
          What the server is configured with and whether each part is working. Values are read from the server
          environment; this page shows only whether each one is set.
        </p>

        {status ? <StatusGrid status={status} transport={transport} /> : <p className="mt-8 text-[13px] text-muted-foreground">Loading…</p>}

        <SettingsForm key={JSON.stringify(settings)} settings={settings} onSave={onSaveSettings} />

        <h2 className="mt-14 text-[20px] font-semibold tracking-[-0.02em]">Connect iMessage</h2>
        <p className="mt-1 text-[14px] text-muted-foreground">
          Pick one transport and set <Code>IMESSAGE_TRANSPORT</Code> to its name.
        </p>
        <div className="mt-5 grid gap-4 lg:grid-cols-2">
          <Guide title="Sendblue" active={status?.transport === "sendblue"} subtitle="Hosted iMessage API. No Mac needed.">
            <Step n={1}>Create a Sendblue account and a number at dashboard.sendblue.com.</Step>
            <Step n={2}>
              Set <Code>SENDBLUE_API_KEY</Code>, <Code>SENDBLUE_API_SECRET</Code>, <Code>SENDBLUE_FROM_NUMBER</Code>, and a
              random <Code>SENDBLUE_WEBHOOK_SECRET</Code> (<Code>openssl rand -hex 32</Code>).
            </Step>
            <Step n={3}>
              In Sendblue, set the receive webhook to this URL, with the same secret:
              <CopyLine value={status?.webhookUrl ?? "https://<your app>/api/webhooks/sendblueWebhook"} />
            </Step>
            <Step n={4}>Text your Sendblue number from a phone on the allowlist.</Step>
          </Guide>
          <Guide title="Mac relay" active={status?.transport === "relay"} subtitle="Your own Mac and Apple ID. Runs relay/ with Bun.">
            <Step n={1}>
              Set <Code>RELAY_TOKEN</Code> on the server (<Code>openssl rand -hex 32</Code>).
            </Step>
            <Step n={2}>
              On an always-on Mac signed in to Messages, copy the project, then create <Code>relay/.env</Code> with{" "}
              <Code>PYLON_URL={status?.relayUrl ?? "https://<your app>"}</Code> and the same <Code>RELAY_TOKEN</Code>.
            </Step>
            <Step n={3}>
              System Settings → Privacy &amp; Security → Full Disk Access: add the terminal app (or <Code>bun</Code>) that runs the
              relay. It reads <Code>~/Library/Messages/chat.db</Code> read-only.
            </Step>
            <Step n={4}>
              Run <Code>bun run relay</Code>. On the first reply, macOS asks to let it control Messages: allow it
              (Privacy &amp; Security → Automation).
            </Step>
          </Guide>
        </div>

        {hasDemo ? (
          <div className="mt-12 flex flex-col gap-3 rounded-[1.2rem] bg-muted/70 p-4 sm:flex-row sm:items-center">
            <p className="flex-1 text-[13px] text-muted-foreground">
              The sample conversations were written by hand to show the layout. Remove them once real texts arrive.
            </p>
            <button
              type="button"
              onClick={() => void onClearDemo()}
              className="rounded-full bg-background px-4 py-2 text-[13px] font-medium ring-1 ring-border"
            >
              Remove sample data
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function StatusGrid({ status, transport }: { status: SetupStatus; transport: TransportStateView | null }) {
  const online = relayOnline(transport?.relayLastSeenAt ?? null);
  const transportOk = status.transport !== null && status.checklist.every((c) => c.ok);
  const needsProdConfig = !status.devMode && !status.appUrlSet;
  return (
    <div className="mt-8 grid gap-3 sm:grid-cols-2">
      <StatusCard
        ok={transportOk && (status.transport !== "relay" || online)}
        title={status.transport === "sendblue" ? "Sendblue" : status.transport === "relay" ? "Mac relay" : "No transport"}
        lines={[
          status.transport === null
            ? "Set IMESSAGE_TRANSPORT to sendblue or relay."
            : status.transport === "relay"
              ? online
                ? `Relay online on ${transport?.relayHost ?? "your Mac"} (v${transport?.relayVersion ?? "?"})`
                : `Relay last seen ${ago(transport?.relayLastSeenAt ?? null)}`
              : `Webhook ready`,
          `Last text in ${ago(transport?.lastInboundAt ?? null)} · last reply out ${ago(transport?.lastOutboundAt ?? null)}`,
          ...(status.dryRun
            ? [
                status.devMode
                  ? "Development server: replies are recorded, not sent. Set IMESSAGE_ALLOW_DEV_SENDS=1 to send."
                  : "IMESSAGE_DRY_RUN is on: replies are recorded, not sent.",
              ]
            : []),
          ...(transport?.lastError ? [`Last error ${ago(transport.lastErrorAt)}: ${transport.lastError}`] : []),
        ]}
      >
        {status.checklist.length > 0 ? (
          <ul className="mt-3 space-y-1.5">
            {status.checklist.map((c) => (
              <li key={c.key} className="flex gap-2 text-[12px] leading-snug">
                <Dot ok={c.ok} />
                <span>
                  <Code>{c.key}</Code>
                  {c.ok ? null : <span className="block text-muted-foreground">{c.hint}</span>}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </StatusCard>
      <StatusCard
        ok={status.provider.configured}
        title="Model"
        lines={[
          status.provider.configured
            ? `${status.provider.provider === "openai" ? "OpenAI" : "Anthropic"} key set`
            : "No model key. Texts are stored and marked waiting until you add ANTHROPIC_API_KEY (or OPENAI_API_KEY with PYLON_LLM_PROVIDER=openai).",
        ]}
      />
      <StatusCard
        ok={status.ownerConfigured}
        title="Owner"
        lines={[
          status.ownerConfigured
            ? "Signed in with a verified email from PYLON_ADMIN_EMAILS. Replies run under this account."
            : "Sign in at /login with an email listed in PYLON_ADMIN_EMAILS.",
        ]}
      />
      <StatusCard
        ok={!needsProdConfig}
        title="Server"
        lines={[
          `Public URL ${status.baseUrl}`,
          ...(!status.devMode && !status.appUrlSet
            ? ["Set APP_URL to this app's public https origin so the webhook and relay URLs are right."]
            : []),
        ]}
      />
    </div>
  );
}

function StatusCard({ ok, title, lines, children }: { ok: boolean; title: string; lines: string[]; children?: React.ReactNode }) {
  return (
    <div className="rounded-[1.3rem] bg-black/[0.03] p-1.5 ring-1 ring-black/[0.04] dark:bg-white/[0.04]">
      <div className="h-full rounded-[calc(1.3rem-0.375rem)] bg-background p-4 shadow-[0_1px_2px_rgba(0,0,0,0.04)]">
        <div className="flex items-center gap-2">
          <Dot ok={ok} />
          <h2 className="text-[15px] font-semibold tracking-[-0.01em]">{title}</h2>
          <span className={cn("ml-auto text-[12px] font-medium", ok ? "text-ok" : "text-warn")}>{ok ? "Ready" : "Needs setup"}</span>
        </div>
        {lines.map((l) => (
          <p key={l} className="mt-1.5 text-[13px] leading-snug text-muted-foreground">
            {l}
          </p>
        ))}
        {children}
      </div>
    </div>
  );
}

function Dot({ ok }: { ok: boolean }) {
  return ok ? (
    <Check className="mt-px size-4 shrink-0 text-ok" strokeWidth={2} aria-label="Set" />
  ) : (
    <Circle className="mt-px size-4 shrink-0 text-warn" strokeWidth={1.75} aria-label="Missing" />
  );
}

function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded-[5px] bg-black/[0.05] px-1 py-px font-mono text-[0.86em] dark:bg-white/[0.08]">{children}</code>;
}

function CopyLine({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <span className="mt-2 flex items-center gap-2 rounded-lg bg-muted px-2.5 py-1.5">
      <code className="min-w-0 flex-1 truncate font-mono text-[12px]">{value}</code>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard?.writeText(value).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          });
        }}
        className="flex items-center gap-1 text-[12px] font-medium text-bubble-out"
      >
        <Copy className="size-3.5" strokeWidth={1.5} />
        {copied ? "Copied" : "Copy"}
      </button>
    </span>
  );
}

function Guide({ title, subtitle, active, children }: { title: string; subtitle: string; active: boolean; children: React.ReactNode }) {
  return (
    <section className={cn("rounded-[1.3rem] p-5 ring-1", active ? "bg-bubble-out/[0.04] ring-bubble-out/30" : "bg-background ring-border")}>
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-[16px] font-semibold tracking-[-0.01em]">{title}</h3>
        {active ? <span className="text-[12px] font-medium text-bubble-out">In use</span> : null}
      </div>
      <p className="mt-0.5 text-[13px] text-muted-foreground">{subtitle}</p>
      <ol className="mt-4 space-y-3">{children}</ol>
    </section>
  );
}

function Step({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="flex gap-3 text-[13px] leading-relaxed">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-foreground text-[11px] font-semibold text-background">
        {n}
      </span>
      <span className="min-w-0 flex-1">{children}</span>
    </li>
  );
}

const TIMEZONES = [
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Phoenix",
  "America/Los_Angeles",
  "America/Anchorage",
  "Pacific/Honolulu",
  "Europe/London",
  "Europe/Berlin",
  "Asia/Tokyo",
  "Australia/Sydney",
  "UTC",
];

function SettingsForm({ settings, onSave }: { settings: SettingsView; onSave: (patch: Partial<SettingsView>) => Promise<void> }) {
  const [draft, setDraft] = useState(settings);
  const [state, setState] = useState<"idle" | "saving" | "saved" | { error: string }>("idle");
  const zones = TIMEZONES.includes(draft.timezone) ? TIMEZONES : [draft.timezone, ...TIMEZONES];
  const set = <K extends keyof SettingsView>(k: K, v: SettingsView[K]) => setDraft((d) => ({ ...d, [k]: v }));

  return (
    <form
      className="mt-12"
      onSubmit={async (e) => {
        e.preventDefault();
        setState("saving");
        try {
          await onSave(draft);
          setState("saved");
        } catch (err) {
          setState({ error: err instanceof Error ? err.message : "Could not save" });
        }
      }}
    >
      <h2 className="text-[20px] font-semibold tracking-[-0.02em]">Assistant settings</h2>
      <div className="mt-5 grid gap-4 rounded-[1.3rem] bg-background p-5 ring-1 ring-border sm:grid-cols-2">
        <Field label="Your name" hint="The assistant refers to you by this name.">
          <input value={draft.ownerName} onChange={(e) => set("ownerName", e.target.value)} placeholder="Jordan" className={inputClass} />
        </Field>
        <Field label="Timezone" hint="Used for reminder times.">
          <select value={draft.timezone} onChange={(e) => set("timezone", e.target.value)} className={inputClass}>
            {zones.map((z) => (
              <option key={z} value={z}>
                {z}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Replies per contact per hour" hint="Texts past this limit are stored, not answered.">
          <input
            type="number"
            min={1}
            max={500}
            value={draft.rateLimitPerHour}
            onChange={(e) => set("rateLimitPerHour", Number(e.target.value))}
            className={inputClass}
          />
        </Field>
        <Field label="Texts from numbers not on the allowlist" hint="Their texts are always stored under Requests.">
          <select
            value={draft.unknownSenderPolicy}
            onChange={(e) => set("unknownSenderPolicy", e.target.value as SettingsView["unknownSenderPolicy"])}
            className={inputClass}
          >
            <option value="ignore">Do not reply</option>
            <option value="reply_once">Send one fixed reply</option>
          </select>
        </Field>
        {draft.unknownSenderPolicy === "reply_once" ? (
          <Field label="Fixed reply" hint="Sent once per number, never generated by the model." wide>
            <textarea
              value={draft.unknownSenderReply}
              onChange={(e) => set("unknownSenderReply", e.target.value)}
              rows={2}
              maxLength={320}
              className={cn(inputClass, "h-auto py-2")}
            />
          </Field>
        ) : null}
        <label className="flex items-center gap-3 sm:col-span-2">
          <input type="checkbox" checked={draft.paused} onChange={(e) => set("paused", e.target.checked)} className="size-4 accent-[#0a84ff]" />
          <span className="text-[13px]">
            <span className="font-medium">Pause the assistant</span>
            <span className="block text-muted-foreground">Texts keep arriving and are stored; nobody gets a reply.</span>
          </span>
        </label>
        <div className="flex items-center gap-3 sm:col-span-2">
          <button
            type="submit"
            disabled={state === "saving"}
            className="rounded-full bg-foreground px-5 py-2 text-[13px] font-medium text-background transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98]"
          >
            Save settings
          </button>
          <span className={cn("text-[12px]", typeof state === "object" ? "text-destructive" : "text-muted-foreground")}>
            {state === "saved" ? "Saved." : typeof state === "object" ? state.error : ""}
          </span>
        </div>
      </div>
    </form>
  );
}

const inputClass = "h-9 w-full rounded-lg border bg-background px-3 text-[14px] outline-none focus:ring-2 focus:ring-ring/30";

function Field({ label, hint, wide, children }: { label: string; hint: string; wide?: boolean; children: React.ReactNode }) {
  return (
    <label className={cn("block", wide && "sm:col-span-2")}>
      <span className="mb-1 block text-[13px] font-medium">{label}</span>
      {children}
      <span className="mt-1 block text-[12px] text-muted-foreground">{hint}</span>
    </label>
  );
}
