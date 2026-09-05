"use client";

import React, { useState } from "react";
import { callFn, db } from "@pylonsync/react";
import { createInvite, createOrg, renameOrg } from "@pylonsync/client";
import { Button } from "@/components/ui/button";
import { TRIAL_DAYS, planById } from "@/lib/plans";

type Step = "workspace" | "team" | "project" | "plan";
const ORDER: Step[] = ["workspace", "team", "project", "plan"];

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10";

/**
 * Four short steps, each one decision, each skippable after the first:
 *
 *   1. Workspace name  → creates the org and makes it the active tenant
 *   2. Invite the team → up to three emails, or skip
 *   3. First project   → so the dashboard is not empty on arrival
 *   4. Plan            → start the Pro trial (Stripe Checkout) or stay free
 *
 * The workspace is created at step 1, so a closed tab still leaves the user
 * with a usable account; the dashboard sends them back here until step 4
 * stamps `Org.onboardedAt`.
 */
export function OnboardingWizard({
  org,
  email,
}: {
  org: { id: string; name: string } | null;
  email: string;
}) {
  const [orgId, setOrgId] = useState<string | null>(org?.id ?? null);
  const [step, setStep] = useState<Step>("workspace");
  const [name, setName] = useState(org?.name ?? suggestName(email));
  const [emails, setEmails] = useState(["", "", ""]);
  const [project, setProject] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const index = ORDER.indexOf(step);

  async function run(fn: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function saveWorkspace(e: React.FormEvent) {
    e.preventDefault();
    const value = name.trim();
    if (!value) return;
    void run(async () => {
      if (orgId) {
        await renameOrg(orgId, value);
      } else {
        const created = await createOrg(value);
        setOrgId(created.id);
        await db.sync.selectOrg(created.id);
      }
      setStep("team");
    });
  }

  function sendInvites() {
    const list = emails.map((x) => x.trim()).filter((x) => x.includes("@"));
    void run(async () => {
      if (orgId) {
        for (const address of list) await createInvite(orgId, address, "member");
      }
      setStep("project");
    });
  }

  function createFirstProject(e: React.FormEvent) {
    e.preventDefault();
    const value = project.trim();
    if (!value || !orgId) return;
    void run(async () => {
      await callFn("createProject", { orgId, name: value });
      setStep("plan");
    });
  }

  async function finish(startTrial: boolean) {
    if (!orgId) return;
    await run(async () => {
      await callFn("completeOnboarding", { orgId });
      if (startTrial) {
        const origin = window.location.origin;
        const res = await callFn<{ url: string }>("createCheckoutSession", {
          plan: "pro",
          referenceId: orgId,
          annual: true,
          successUrl: `${origin}/dashboard?welcome=1`,
          cancelUrl: `${origin}/dashboard/billing`,
        });
        window.location.assign(res.url);
        return;
      }
      window.location.assign("/dashboard?welcome=1");
    });
  }

  const pro = planById("pro")!;

  return (
    <div className="flex min-h-screen items-center justify-center bg-zinc-50 px-6 py-12">
      <div className="w-full max-w-md">
        <div className="mb-6 flex items-center gap-2">
          {ORDER.map((s, i) => (
            <span
              key={s}
              className={`h-1.5 flex-1 rounded-full ${i <= index ? "bg-zinc-900" : "bg-zinc-200"}`}
            />
          ))}
        </div>
        <div className="rounded-2xl border border-zinc-200 bg-white p-8">
          {step === "workspace" && (
            <form onSubmit={saveWorkspace} className="space-y-5">
              <div>
                <h1 className="text-xl font-semibold tracking-tight text-zinc-900">Name your workspace</h1>
                <p className="mt-1 text-sm text-zinc-500">Usually your company or team name. You can change it later.</p>
              </div>
              <input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Acme Inc"
                aria-label="Workspace name"
                className={inputCls}
              />
              <Button type="submit" className="w-full" disabled={busy || !name.trim()}>
                {busy ? "…" : "Continue"}
              </Button>
            </form>
          )}

          {step === "team" && (
            <div className="space-y-5">
              <div>
                <h1 className="text-xl font-semibold tracking-tight text-zinc-900">Invite your team</h1>
                <p className="mt-1 text-sm text-zinc-500">Teammates get an email with a link to join. Add more later from Members.</p>
              </div>
              <div className="space-y-2">
                {emails.map((value, i) => (
                  <input
                    key={i}
                    type="email"
                    value={value}
                    onChange={(e) => setEmails(emails.map((x, j) => (j === i ? e.target.value : x)))}
                    placeholder={i === 0 ? "teammate@company.com" : "Another email (optional)"}
                    aria-label={`Invite email ${i + 1}`}
                    className={inputCls}
                  />
                ))}
              </div>
              <div className="flex gap-2">
                <Button type="button" variant="outline" className="flex-1" onClick={() => setStep("project")} disabled={busy}>
                  Skip for now
                </Button>
                <Button type="button" className="flex-1" onClick={sendInvites} disabled={busy}>
                  {busy ? "…" : "Send invites"}
                </Button>
              </div>
            </div>
          )}

          {step === "project" && (
            <form onSubmit={createFirstProject} className="space-y-5">
              <div>
                <h1 className="text-xl font-semibold tracking-tight text-zinc-900">Create your first project</h1>
                <p className="mt-1 text-sm text-zinc-500">Something you are working on this week. It shows up on your dashboard.</p>
              </div>
              <input
                autoFocus
                value={project}
                onChange={(e) => setProject(e.target.value)}
                placeholder="Website redesign"
                aria-label="Project name"
                className={inputCls}
              />
              <div className="flex gap-2">
                <Button type="button" variant="outline" className="flex-1" onClick={() => setStep("plan")} disabled={busy}>
                  Skip
                </Button>
                <Button type="submit" className="flex-1" disabled={busy || !project.trim()}>
                  {busy ? "…" : "Create project"}
                </Button>
              </div>
            </form>
          )}

          {step === "plan" && (
            <div className="space-y-5">
              <div>
                <h1 className="text-xl font-semibold tracking-tight text-zinc-900">Pick a plan</h1>
                <p className="mt-1 text-sm text-zinc-500">
                  Try Pro free for {TRIAL_DAYS} days. Cancel any time from Billing; you keep the free plan.
                </p>
              </div>
              <ul className="space-y-2 rounded-xl border border-zinc-200 bg-zinc-50 p-4 text-sm text-zinc-700">
                {pro.features.map((f) => (
                  <li key={f} className="flex gap-2">
                    <span className="text-zinc-900">✓</span> {f}
                  </li>
                ))}
              </ul>
              <Button type="button" className="w-full" onClick={() => void finish(true)} disabled={busy}>
                {busy ? "…" : pro.cta}
              </Button>
              <button
                type="button"
                onClick={() => void finish(false)}
                disabled={busy}
                className="w-full text-center text-[13px] text-zinc-500 underline underline-offset-2 hover:text-zinc-900"
              >
                Continue on the free plan
              </button>
            </div>
          )}

          {error && (
            <p className="mt-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[13px] text-red-700">
              {error.replace(/^[A-Z_]+:\s*/, "")}
            </p>
          )}
        </div>
        <p className="mt-4 text-center text-xs text-zinc-400">
          Step {index + 1} of {ORDER.length}
        </p>
      </div>
    </div>
  );
}

/** "jane@northwind.com" → "Northwind". A starting point the user can edit. */
function suggestName(email: string): string {
  const domain = email.split("@")[1]?.split(".")[0] ?? "";
  if (!domain || ["gmail", "yahoo", "outlook", "hotmail", "icloud", "proton", "me"].includes(domain)) {
    return "";
  }
  return domain.charAt(0).toUpperCase() + domain.slice(1);
}
