"use client";

import React from "react";
import type { ErrorBoundaryProps } from "@pylonsync/react";

export default function ErrorPage({ error, reset }: ErrorBoundaryProps) {
  return (
    <main className="flex min-h-[100dvh] flex-col items-center justify-center gap-3 px-4 text-center">
      <h1 className="text-[22px] font-semibold tracking-[-0.02em]">Something went wrong</h1>
      <p className="max-w-[48ch] text-[14px] text-muted-foreground">{error.message}</p>
      <button type="button" onClick={reset} className="rounded-full bg-foreground px-5 py-2 text-[13px] font-medium text-background">
        Try again
      </button>
    </main>
  );
}
