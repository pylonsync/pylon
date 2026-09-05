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
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">Reset your password</h1>
      <p className="mt-1 text-[13px] text-zinc-500">Enter your email and we will send a reset link.</p>
      <div className="mt-6">
        <ForgotPasswordForm />
      </div>
    </>
  );
}
