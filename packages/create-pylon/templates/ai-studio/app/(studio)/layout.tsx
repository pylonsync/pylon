import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// `(studio)` is a ROUTE GROUP: the parens segment is stripped from every URL
// (so `(studio)/page.tsx` still serves `/`), and this layout wraps only the
// pages inside the group. /login sits outside it and renders bare in the
// root shell.
//
// App shell: a slim top bar over the studio. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the bar shows
// the account / "Sign in" with no flash. The header is h-14; the studio page
// fills the rest of the viewport (min-h-[calc(100vh-3.5rem)]).
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

export default function StudioLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  const { brand } = siteConfig;

  return (
    <>
      <header className="flex h-14 items-center justify-between border-b border-zinc-200 bg-white px-4">
        <Link href="/" className="flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-[7px] bg-brand text-[13px] font-bold text-white">
            {brand.letter}
          </span>
          <span className="text-[15px] font-semibold tracking-tight text-zinc-900">{brand.name}</span>
        </Link>
        {signedIn ? (
          <span className="text-[13px] text-zinc-400">Signed in</span>
        ) : (
          <Link
            href="/login"
            className="rounded-full border border-zinc-300 px-3.5 py-1.5 text-[13px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50"
          >
            Sign in
          </Link>
        )}
      </header>
      {children}
    </>
  );
}
