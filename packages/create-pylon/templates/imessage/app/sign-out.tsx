"use client";

import React from "react";
import { useAuth } from "@pylonsync/client";

export function SignOutAndRetry() {
  const { signOut } = useAuth();
  return (
    <button
      type="button"
      onClick={async () => {
        await signOut();
        window.location.assign("/login");
      }}
      className="mt-6 rounded-full bg-foreground px-5 py-2 text-[13px] font-medium text-background"
    >
      Sign in with another email
    </button>
  );
}
