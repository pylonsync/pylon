import React from "react";
import { Link } from "@pylonsync/react";

// Root not-found boundary. Rendered (wrapped in RootLayout) whenever a
// page calls `response.notFound()` and no closer not-found.tsx exists.
export default function NotFound() {
  return (
    <div className="space-y-4">
      <h1 className="text-3xl font-semibold tracking-tight text-rose-600">
        404 — Custom Not Found
      </h1>
      <p className="text-zinc-600">
        Rendered by <code>app/not-found.tsx</code>, wrapped in the root layout.
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
