import React from "react";
import type { PageProps } from "@pylonsync/react";
import { SignOutAndRetry } from "./sign-out";

/**
 * Server-side gate shared by the dashboard pages. Anonymous visitors and guest
 * sessions go to /login. A signed-in account that is not the owner (not an
 * admin via PYLON_ADMIN_EMAILS) gets an explanation and no data: every entity
 * policy denies non-admins as well, so this is presentation, not the fence.
 */
export function ownerGate({ auth, response }: Pick<PageProps, "auth" | "response">): React.ReactNode | null {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return <></>;
  }
  if (!auth.is_admin) {
    return (
      <main className="flex min-h-[100dvh] items-center justify-center px-4">
        <div className="max-w-[420px] text-center">
          <h1 className="text-[22px] font-semibold tracking-[-0.02em]">This dashboard belongs to its owner</h1>
          <p className="mt-2 text-[14px] leading-relaxed text-muted-foreground">
            You are signed in, but your email is not the owner email set in PYLON_ADMIN_EMAILS on this server, or it has
            not been verified with a sign-in code yet.
          </p>
          <SignOutAndRetry />
        </div>
      </main>
    );
  }
  return null;
}
