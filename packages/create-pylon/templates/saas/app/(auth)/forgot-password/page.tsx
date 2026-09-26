import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ForgotPasswordForm } from "../recovery-forms";

export const metadata: Metadata = {
  title: "Reset your password — Acme",
  robots: "noindex",
};

export default function ForgotPasswordPage() {
  return (
    <>
      <h1 className="mt-10 text-[26px] font-semibold tracking-[-0.025em] text-zinc-950">Reset your password</h1>
      <p className="mt-1 text-[13px] text-zinc-500">Enter your email and we will send a reset link.</p>
      <div className="mt-6">
        <ForgotPasswordForm />
      </div>
    </>
  );
}
