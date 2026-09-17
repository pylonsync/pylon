"use client";

import React, { useState } from "react";
import { callFn } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { LINK } from "@/components/marketing";

// The "submit a tool" form. submitListing is a public mutation, so it works for
// anonymous visitors with no session. PRIVACY: the submission (incl. the
// submitter's email) is written to the deny-all Submission table. It is NOT
// public and never syncs to a browser. It shows up in the directory only after
// the owner approves it from the dashboard.
export function SubmitForm() {
  const { submit, categories } = siteConfig;
  const [form, setForm] = useState({
    submitterName: "",
    submitterEmail: "",
    name: "",
    tagline: "",
    url: "",
    category: categories[0] ?? "",
    tags: "",
    description: "",
  });
  const [status, setStatus] = useState<"idle" | "sending" | "done">("idle");
  const [error, setError] = useState<string | null>(null);

  const set =
    (k: keyof typeof form) =>
    (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) =>
      setForm((f) => ({ ...f, [k]: e.target.value }));

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (status === "sending") return;
    if (!form.submitterName.trim() || !form.submitterEmail.trim() || !form.name.trim() || !form.url.trim()) {
      setError("Your name, your email, the tool name, and the URL are required.");
      return;
    }
    setStatus("sending");
    setError(null);
    try {
      await callFn<{ ok: boolean }>("submitListing", {
        submitterName: form.submitterName.trim(),
        submitterEmail: form.submitterEmail.trim(),
        name: form.name.trim(),
        tagline: form.tagline.trim(),
        url: form.url.trim(),
        category: form.category,
        tags: form.tags.trim() || undefined,
        description: form.description.trim() || undefined,
      });
      setStatus("done");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        /valid email/i.test(msg)
          ? "Enter a valid email address."
          : /valid URL/i.test(msg)
            ? "Enter a valid URL (https://…)."
            : /INVALID_ARGS/i.test(msg)
              ? "Fill in the required fields."
              : "The submission failed. Try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="border-t border-zinc-200 pt-4">
        <p className="text-[14px] font-medium text-zinc-900">{submit.confirmationMessage}</p>
        <a href="/" className={`${LINK} mt-2 inline-block text-[14px]`}>
          Back to the directory
        </a>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="border-t border-zinc-200 pt-5">
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="Your name" required>
          <input value={form.submitterName} onChange={set("submitterName")} autoComplete="name" className={inputCls} />
        </Field>
        <Field label="Your email" required>
          <input type="email" value={form.submitterEmail} onChange={set("submitterEmail")} autoComplete="email" className={inputCls} />
        </Field>
      </div>

      <div className="mt-5 border-t border-zinc-200 pt-5">
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="Tool name" required>
            <input value={form.name} onChange={set("name")} className={inputCls} />
          </Field>
          <Field label="Category">
            <select value={form.category} onChange={set("category")} className={inputCls}>
              {categories.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
          </Field>
        </div>
        <div className="mt-4">
          <Field label="URL" required>
            <input value={form.url} onChange={set("url")} placeholder="https://" inputMode="url" className={`${inputCls} font-mono text-[13px]`} />
          </Field>
        </div>
        <div className="mt-4">
          <Field label="One-line description">
            <input value={form.tagline} onChange={set("tagline")} placeholder="What it does, in one sentence." className={inputCls} />
          </Field>
        </div>
        <div className="mt-4">
          <Field label="Tags">
            <input value={form.tags} onChange={set("tags")} placeholder="comma, separated" className={`${inputCls} font-mono text-[13px]`} />
          </Field>
        </div>
        <div className="mt-4">
          <Field label="Notes for the reviewer">
            <textarea value={form.description} onChange={set("description")} rows={3} className={`${inputCls} h-auto resize-none py-2`} />
          </Field>
        </div>
      </div>

      {error ? <p className="mt-4 text-[13px] text-red-700">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "sending"}
        className="mt-6 h-10 border border-brand bg-brand px-5 text-[14px] font-medium text-white hover:opacity-90 disabled:opacity-60"
      >
        {status === "sending" ? "Submitting…" : "Submit for review"}
      </button>
    </form>
  );
}

function Field({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[12px] font-medium uppercase tracking-wide text-zinc-500">
        {label}
        {required ? <span className="text-brand"> *</span> : null}
      </span>
      {children}
    </label>
  );
}

const inputCls =
  "h-10 w-full border border-zinc-300 bg-white px-3 text-[14px] text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-brand";
