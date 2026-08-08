"use client";

import React, { useState } from "react";
import { callFn } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// The "submit a tool" form. submitListing is a public mutation, so it works for
// anonymous visitors with no session. PRIVACY: the submission (incl. the
// submitter's email) is written to the deny-all Submission table — it is NOT
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
      setError("Your name + email and the tool's name + URL are required.");
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
              ? "Please fill in the required fields."
              : "Something went wrong — try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="rounded-2xl border border-brand/30 bg-brand-soft/50 px-6 py-10 text-center">
        <div className="mx-auto flex size-10 items-center justify-center rounded-full bg-brand text-white">✓</div>
        <p className="mt-3 text-[15px] font-semibold text-zinc-900">{submit.confirmationMessage}</p>
        <a href="/" className="mt-5 inline-flex h-10 items-center rounded-full border border-zinc-300 px-5 text-sm font-medium text-zinc-700 hover:bg-zinc-50">
          Back to the directory
        </a>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="rounded-2xl border border-zinc-200 bg-white p-6 shadow-sm sm:p-7">
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Your name" required>
          <input value={form.submitterName} onChange={set("submitterName")} autoComplete="name" className={inputCls} />
        </Field>
        <Field label="Your email" required>
          <input type="email" value={form.submitterEmail} onChange={set("submitterEmail")} autoComplete="email" className={inputCls} />
        </Field>
      </div>

      <div className="mt-3 border-t border-zinc-100 pt-4">
        <div className="grid gap-3 sm:grid-cols-2">
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
        <div className="mt-3">
          <Field label="URL" required>
            <input value={form.url} onChange={set("url")} placeholder="https://…" inputMode="url" className={inputCls} />
          </Field>
        </div>
        <div className="mt-3">
          <Field label="One-line tagline">
            <input value={form.tagline} onChange={set("tagline")} placeholder="What it does, in a sentence." className={inputCls} />
          </Field>
        </div>
        <div className="mt-3">
          <Field label="Tags">
            <input value={form.tags} onChange={set("tags")} placeholder="comma, separated, tags" className={inputCls} />
          </Field>
        </div>
        <div className="mt-3">
          <Field label="Anything else?">
            <textarea value={form.description} onChange={set("description")} rows={3} className={inputCls + " resize-none py-2.5"} />
          </Field>
        </div>
      </div>

      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "sending"}
        className="mt-5 inline-flex h-11 w-full items-center justify-center rounded-full bg-brand text-[15px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60 sm:w-auto sm:px-7"
      >
        {status === "sending" ? "Submitting…" : "Submit for review"}
      </button>
    </form>
  );
}

function Field({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[12.5px] font-medium text-zinc-600">
        {label}
        {required ? <span className="text-brand"> *</span> : null}
      </span>
      {children}
    </label>
  );
}

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-[14px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20";
