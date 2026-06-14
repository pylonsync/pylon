import React from "react";
import { Link } from "@pylonsync/react";

// Server-rendered split-screen shell for /login and /signup: the form on the
// left, a brand/testimonial panel on the right (hidden on small screens). The
// form itself (the client island) is passed in as `children`.
export function AuthShell({
  title,
  switchPrompt,
  switchLabel,
  switchHref,
  children,
}: {
  title: string;
  switchPrompt: string;
  switchLabel: string;
  switchHref: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid min-h-screen bg-white lg:grid-cols-2">
      {/* Form side */}
      <div className="flex items-center justify-center px-6 py-12">
        <div className="w-full max-w-[400px] rounded-2xl border border-zinc-200/70 p-8">
          <Link href="/" className="inline-flex">
            <span className="flex size-9 items-center justify-center rounded-xl bg-zinc-900 text-base font-bold text-white">
              A
            </span>
          </Link>
          <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">
            {title}
          </h1>
          <p className="mt-1 text-[13px] text-zinc-500">
            {switchPrompt}{" "}
            <Link
              href={switchHref}
              className="font-medium text-zinc-900 underline underline-offset-2"
            >
              {switchLabel}
            </Link>
          </p>
          <div className="mt-6">{children}</div>
        </div>
      </div>

      {/* Brand / testimonial side */}
      <div className="relative hidden flex-col justify-center bg-zinc-50 px-14 lg:flex">
        <div className="max-w-md">
          <div className="font-serif text-5xl leading-none text-zinc-300">
            &ldquo;
          </div>
          <blockquote className="mt-2 text-[1.6rem] font-medium leading-snug tracking-tight text-zinc-900">
            Our whole team finally works in one place. Acme makes it easy to
            plan, build, and ship without the busywork.
          </blockquote>
          <div className="mt-8 flex items-center gap-3">
            <span className="flex size-10 items-center justify-center rounded-full bg-zinc-200 text-[13px] font-semibold text-zinc-500">
              MC
            </span>
            <div className="leading-tight">
              <div className="text-sm font-semibold text-zinc-900">
                Maya Chen
              </div>
              <div className="text-[13px] text-zinc-500">
                Head of Product, Northwind
              </div>
            </div>
          </div>
        </div>
        <p className="absolute bottom-8 left-14 text-[13px] text-zinc-400">
          Projects, docs, and automation. All in one place.
        </p>
      </div>
    </div>
  );
}
