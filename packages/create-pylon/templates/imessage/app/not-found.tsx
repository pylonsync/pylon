import React from "react";
import { Link } from "@pylonsync/react";

export default function NotFound() {
  return (
    <main className="flex min-h-[100dvh] flex-col items-center justify-center gap-3 px-4 text-center">
      <h1 className="text-[22px] font-semibold tracking-[-0.02em]">Page not found</h1>
      <Link href="/" className="text-[14px] font-medium text-bubble-out">
        Back to messages
      </Link>
    </main>
  );
}
