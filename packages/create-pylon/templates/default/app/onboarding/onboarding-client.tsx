"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import {
  CreateOrganization,
  InviteMembers,
  useAuth,
  type OrgSummary,
} from "@pylonsync/client";
import { Button } from "@/components/ui/button";

// First-run onboarding. Three steps, each built on a real @pylonsync/client
// primitive so the flow does actual work end-to-end (no mock screens):
//   1. Workspace  → <CreateOrganization> (createOrg) + selectOrg to make it the
//                   active tenant.
//   2. Teammates  → <InviteMembers> (POST /api/auth/orgs/:id/invites, gated to
//                   admins by the framework — the creator is the admin).
//   3. Project    → db.insert("Project", { orgId }) — the same tenant-scoped,
//                   optimistic write the dashboard uses.
// The marketing nav/footer are suppressed for /onboarding in the root layout,
// so this owns the whole screen. The client components theme off --pylon-* vars,
// which we map to the template's zinc palette on the wrapper below so they blend.
const STEPS = ["Workspace", "Invite", "Project"] as const;

export function Onboarding({ email }: { email: string }) {
  const { selectOrg } = useAuth();
  const [step, setStep] = useState(0);
  const [org, setOrg] = useState<OrgSummary | null>(null);

  async function onOrgCreated(created: OrgSummary) {
    // CreateOrganization creates the org but doesn't switch into it — make it
    // the active tenant so the invite + project steps (and the dashboard after)
    // operate on it.
    await selectOrg(created.id);
    setOrg(created);
    setStep(1);
  }

  function finish() {
    // Full navigation so the SSR dashboard re-renders with the now-active tenant
    // resolved from the session cookie.
    window.location.assign("/dashboard");
  }

  return (
    <div
      className="flex min-h-screen flex-col bg-zinc-50"
      style={
        {
          "--pylon-ink": "#18181b",
          "--pylon-ink-2": "#52525b",
          "--pylon-ink-3": "#a1a1aa",
          "--pylon-paper": "#ffffff",
          "--pylon-paper-2": "#f4f4f5",
          "--pylon-rule": "#e4e4e7",
        } as React.CSSProperties
      }
    >
      <header className="flex h-14 items-center justify-between px-6">
        <a href="/" className="flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
            A
          </span>
          <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
            Acme
          </span>
        </a>
        <span className="truncate text-[13px] text-zinc-500">{email}</span>
      </header>

      <main className="flex flex-1 items-center justify-center px-6 py-10">
        <div className="w-full max-w-md">
          <Stepper current={step} />

          <div className="mt-7 rounded-2xl border border-zinc-200 bg-white p-7 shadow-[0_24px_60px_-32px_rgba(0,0,0,0.25)]">
            {step === 0 && (
              <Step
                eyebrow="Step 1 of 3"
                title="Create your workspace"
                subtitle="A workspace is an isolated tenant — its projects and members are private to it. You can make more later."
              >
                <CreateOrganization title="" onCreated={onOrgCreated} />
              </Step>
            )}

            {step === 1 && org && (
              <Step
                eyebrow="Step 2 of 3"
                title="Invite your team"
                subtitle={`Add people to ${org.name}. They'll get an email invite — skip this if you're flying solo for now.`}
              >
                <InviteMembers orgId={org.id} hideMembers hidePending />
                <StepNav onBack={() => setStep(0)} onNext={() => setStep(2)} />
              </Step>
            )}

            {step === 2 && org && (
              <Step
                eyebrow="Step 3 of 3"
                title="Create your first project"
                subtitle="Projects hold your work. Add one to land in the dashboard with something already there."
              >
                <FirstProject orgId={org.id} onDone={finish} />
              </Step>
            )}
          </div>

          {step === 0 && (
            <p className="mt-5 text-center text-[13px] text-zinc-400">
              Wrong account?{" "}
              <a href="/login" className="text-zinc-600 underline">
                Switch
              </a>
            </p>
          )}
        </div>
      </main>
    </div>
  );
}

function Stepper({ current }: { current: number }) {
  return (
    <ol className="flex items-center gap-2">
      {STEPS.map((label, i) => {
        const state =
          i < current ? "done" : i === current ? "active" : "todo";
        return (
          <li key={label} className="flex flex-1 items-center gap-2">
            <span
              className={
                "flex size-6 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold transition-colors " +
                (state === "active"
                  ? "bg-zinc-900 text-white"
                  : state === "done"
                    ? "bg-brand text-white"
                    : "bg-zinc-200 text-zinc-500")
              }
            >
              {state === "done" ? "✓" : i + 1}
            </span>
            <span
              className={
                "text-[12px] font-medium " +
                (state === "todo" ? "text-zinc-400" : "text-zinc-700")
              }
            >
              {label}
            </span>
            {i < STEPS.length - 1 && (
              <span className="ml-1 h-px flex-1 bg-zinc-200" />
            )}
          </li>
        );
      })}
    </ol>
  );
}

function Step({
  eyebrow,
  title,
  subtitle,
  children,
}: {
  eyebrow: string;
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <p className="text-[11px] font-semibold uppercase tracking-wide text-brand">
        {eyebrow}
      </p>
      <h1 className="mt-1.5 text-xl font-semibold tracking-tight text-zinc-900">
        {title}
      </h1>
      <p className="mt-1.5 text-[13.5px] leading-relaxed text-zinc-500">
        {subtitle}
      </p>
      <div className="mt-5">{children}</div>
    </div>
  );
}

function StepNav({
  onBack,
  onNext,
}: {
  onBack: () => void;
  onNext: () => void;
}) {
  return (
    <div className="mt-5 flex items-center justify-between">
      <button
        type="button"
        onClick={onBack}
        className="text-[13px] font-medium text-zinc-500 transition-colors hover:text-zinc-900"
      >
        ← Back
      </button>
      <Button type="button" size="sm" onClick={onNext}>
        Continue
      </Button>
    </div>
  );
}

function FirstProject({
  orgId,
  onDone,
}: {
  orgId: string;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);

  async function create() {
    const value = name.trim();
    if (!value) return;
    setSaving(true);
    await db.insert("Project", { orgId, name: value });
    onDone();
  }

  return (
    <div>
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void create();
        }}
        placeholder="e.g. Website redesign"
        aria-label="Project name"
        autoFocus
        className="h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10"
      />
      <div className="mt-5 flex items-center justify-between">
        <button
          type="button"
          onClick={onDone}
          className="text-[13px] font-medium text-zinc-500 transition-colors hover:text-zinc-900"
        >
          Skip for now
        </button>
        <Button
          type="button"
          size="sm"
          onClick={create}
          disabled={saving || !name.trim()}
        >
          {saving ? "…" : "Finish →"}
        </Button>
      </div>
    </div>
  );
}
