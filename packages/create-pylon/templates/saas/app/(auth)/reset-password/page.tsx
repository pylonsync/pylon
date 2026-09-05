import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ResetPasswordForm } from "../recovery-forms";

export const metadata: Metadata = {
  title: "Choose a new password — Acme",
  robots: "noindex",
};

// The reset email links here with `?token=…` (see PYLON_PUBLIC_URL on the
// server: it builds `<public url>/reset-password?token=…`).
export default function ResetPasswordPage({ searchParams }: PageProps) {
  const raw = (searchParams as Record<string, string | string[] | undefined> | undefined)?.token;
  const token = Array.isArray(raw) ? raw[0] ?? "" : raw ?? "";
  return (
    <>
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">Choose a new password</h1>
      <p className="mt-1 text-[13px] text-zinc-500">You will be signed in on every device after this.</p>
      <div className="mt-6">
        <ResetPasswordForm token={token} />
      </div>
    </>
  );
}
