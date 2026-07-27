import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { AuthForm } from "./auth-form";

export const metadata: Metadata = {
  title: "Sign in",
  robots: "noindex",
};

export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in? Nothing to do here.
  if (auth.user_id && !auth.user_id.startsWith("guest_")) {
    response.redirect("/");
    return null;
  }

  return (
    <main className="flex min-h-screen items-center justify-center px-6">
      <div className="w-full max-w-[320px]">
        <div className="mb-7 flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-md bg-primary text-[12px] font-bold text-primary-foreground">
            C
          </span>
          <span className="text-[15px] font-semibold tracking-tight">CRM</span>
        </div>
        <h1 className="text-[19px] font-semibold tracking-tight">
          Sign in to your workspace
        </h1>
        <p className="mt-1 text-[13px] text-muted-foreground">
          Your team shares one pipeline. Anyone with an account sees it.
        </p>
        <div className="mt-6">
          <AuthForm />
        </div>
      </div>
    </main>
  );
}
