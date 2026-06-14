import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";

// SEO metadata. Exported `metadata` is rendered into <head> on the server, so
// this marketing page is fully indexable — view source and the copy is in the
// HTML. Swap "Acme" for your product throughout.
export const metadata: Metadata = {
  title: "Acme — the workspace your team actually wants",
  description:
    "Acme is a collaborative workspace for fast-moving teams. Organize projects by team, collaborate in real time, and keep every tenant's data private. Built on Pylon.",
};

// `app/page.tsx` → `/`. A server-rendered marketing landing page. It reads
// `auth` (resolved from the session cookie during the render) so the call to
// action is right on the first byte — "Get started" for visitors, "Open
// dashboard" once you're signed in. No client fetch, no flash.
export default function LandingPage({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id);

  return (
    <div className="space-y-24 pb-24">
      {/* Hero */}
      <section className="relative overflow-hidden">
        {/* Soft gradient wash behind the hero. */}
        <div
          aria-hidden
          className="pointer-events-none absolute -top-40 left-1/2 h-[32rem] w-[60rem] -translate-x-1/2 rounded-full bg-gradient-to-tr from-primary/15 via-primary/5 to-transparent blur-3xl"
        />
        <div className="relative mx-auto max-w-3xl px-2 pt-16 text-center sm:pt-24">
          <span className="inline-flex items-center gap-2 rounded-full border bg-background/60 px-3 py-1 text-xs font-medium text-muted-foreground backdrop-blur">
            <span className="size-1.5 rounded-full bg-emerald-500" />
            New · Real-time projects for every team
          </span>
          <h1 className="mt-6 text-balance text-5xl font-semibold tracking-tight sm:text-6xl">
            The workspace your team{" "}
            <span className="bg-gradient-to-r from-primary to-primary/60 bg-clip-text text-transparent">
              actually wants
            </span>
            .
          </h1>
          <p className="mx-auto mt-5 max-w-xl text-balance text-lg text-muted-foreground">
            Acme keeps your team&apos;s projects organized, in sync, and private
            to your organization. Invite your teammates, switch between orgs, and
            watch every change land live.
          </p>
          <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
            {signedIn ? (
              <Button asChild size="lg">
                <Link href="/dashboard">Open dashboard →</Link>
              </Button>
            ) : (
              <>
                <Button asChild size="lg">
                  <Link href="/signup">Get started — it&apos;s free</Link>
                </Button>
                <Button asChild size="lg" variant="outline">
                  <Link href="/login">Sign in</Link>
                </Button>
              </>
            )}
          </div>
          <p className="mt-4 text-xs text-muted-foreground">
            No credit card required · Set up your first org in seconds.
          </p>
        </div>

        {/* Product peek — a stylized dashboard mock so the hero has a subject. */}
        <div className="relative mx-auto mt-14 max-w-4xl px-4">
          <div className="rounded-xl border bg-card shadow-2xl shadow-primary/5">
            <div className="flex items-center gap-1.5 border-b px-4 py-3">
              <span className="size-3 rounded-full bg-red-400/70" />
              <span className="size-3 rounded-full bg-yellow-400/70" />
              <span className="size-3 rounded-full bg-green-400/70" />
              <span className="ml-3 text-xs text-muted-foreground">
                acme.app/dashboard
              </span>
            </div>
            <div className="grid gap-4 p-5 sm:grid-cols-[1fr_1.4fr]">
              <div className="space-y-2">
                <div className="text-xs font-medium text-muted-foreground">
                  Organization
                </div>
                <div className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm">
                  <span className="flex size-6 items-center justify-center rounded bg-primary/10 text-xs font-semibold text-primary">
                    A
                  </span>
                  Acme Inc
                </div>
                <div className="pt-2 text-xs font-medium text-muted-foreground">
                  Members
                </div>
                {["you · owner", "jordan · admin", "sam · member"].map((m) => (
                  <div
                    key={m}
                    className="rounded-md border px-3 py-1.5 font-mono text-xs text-muted-foreground"
                  >
                    {m}
                  </div>
                ))}
              </div>
              <div className="space-y-2">
                <div className="text-xs font-medium text-muted-foreground">
                  Projects
                </div>
                {["Website redesign", "Mobile app", "Q3 launch"].map((p, i) => (
                  <div
                    key={p}
                    className="flex items-center justify-between rounded-md border px-3 py-2 text-sm"
                  >
                    {p}
                    {i === 0 && (
                      <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600">
                        live
                      </span>
                    )}
                  </div>
                ))}
                <div className="rounded-md border border-dashed px-3 py-2 text-sm text-muted-foreground">
                  + New project
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* Logos / social proof */}
      <section className="mx-auto max-w-3xl px-4 text-center">
        <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">
          Trusted by teams at
        </p>
        <div className="mt-4 flex flex-wrap items-center justify-center gap-x-10 gap-y-3 text-lg font-semibold text-muted-foreground/60">
          <span>Globex</span>
          <span>Initech</span>
          <span>Hooli</span>
          <span>Soylent</span>
          <span>Stark</span>
        </div>
      </section>

      {/* Features */}
      <section className="mx-auto max-w-4xl px-4">
        <div className="mx-auto max-w-2xl text-center">
          <h2 className="text-3xl font-semibold tracking-tight">
            Everything a growing team needs
          </h2>
          <p className="mt-3 text-muted-foreground">
            Acme is multi-tenant from day one — every org is an isolated,
            real-time workspace.
          </p>
        </div>
        <div className="mt-12 grid gap-6 sm:grid-cols-3">
          <Feature title="Organize by org" icon="▦">
            Spin up an organization, invite your team, and switch between them in
            a click. Each org&apos;s projects and members are completely private.
          </Feature>
          <Feature title="Real-time by default" icon="✦">
            Changes sync instantly across everyone&apos;s screens — no refresh,
            no stale data. Open two tabs and watch it happen.
          </Feature>
          <Feature title="Secure tenant isolation" icon="◆">
            Access is enforced at the data layer. A member of one org can&apos;t
            read or write another&apos;s rows — not by convention, by policy.
          </Feature>
        </div>
      </section>

      {/* CTA */}
      <section className="mx-auto max-w-3xl px-4">
        <div className="rounded-2xl border bg-gradient-to-br from-primary/10 to-transparent px-8 py-12 text-center">
          <h2 className="text-3xl font-semibold tracking-tight">
            Ready to get your team in sync?
          </h2>
          <p className="mx-auto mt-3 max-w-md text-muted-foreground">
            Create your organization and invite your first teammate in under a
            minute.
          </p>
          <div className="mt-6">
            <Button asChild size="lg">
              <Link href={signedIn ? "/dashboard" : "/signup"}>
                {signedIn ? "Open dashboard →" : "Start for free →"}
              </Link>
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}

function Feature({
  title,
  icon,
  children,
}: {
  title: string;
  icon: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border bg-card p-5">
      <div className="flex size-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
        {icon}
      </div>
      <h3 className="mt-4 text-base font-semibold tracking-tight">{title}</h3>
      <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">
        {children}
      </p>
    </div>
  );
}
