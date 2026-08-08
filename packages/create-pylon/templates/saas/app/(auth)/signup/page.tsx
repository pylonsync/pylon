import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Create your account — Acme",
  robots: "noindex",
};

// `(auth)/signup/page.tsx` → `/signup`. Register mode of the same form; the
// `(auth)` layout supplies the frame and the signed-in redirect.
export default function SignupPage() {
  return (
    <>
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">
        Create an account
      </h1>
      <p className="mt-1 text-[13px] text-zinc-500">
        Already have an account?{" "}
        <Link
          href="/login"
          className="font-medium text-zinc-900 underline underline-offset-2"
        >
          Sign in
        </Link>
      </p>
      <div className="mt-6">
        <AuthForm mode="signup" />
      </div>
    </>
  );
}
