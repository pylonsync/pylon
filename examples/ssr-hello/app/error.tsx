import React from "react";
import { Link } from "@pylonsync/react";

interface ErrorProps {
  error?: { message?: string };
}

// Root error boundary. Rendered (wrapped in RootLayout) at HTTP 500 whenever
// a page/layout throws during the synchronous shell render. The thrown error
// is passed in props.error.
export default function ErrorBoundary({ error }: ErrorProps) {
  return (
    <div className="space-y-4">
      <h1 className="text-3xl font-semibold tracking-tight text-rose-600">
        500 — Custom Error
      </h1>
      <p className="text-zinc-600">
        Rendered by <code>app/error.tsx</code>, wrapped in the root layout.
      </p>
      <p className="rounded-md bg-rose-50 px-3 py-2 text-sm text-rose-700">
        {error?.message ?? "unknown error"}
      </p>
      <p className="text-sm">
        <Link
          href="/"
          className="text-blue-600 underline-offset-4 hover:underline"
        >
          ← home
        </Link>
      </p>
    </div>
  );
}
