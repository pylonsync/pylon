import Link from "next/link";
import { ArrowRight, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { pylon } from "@/lib/pylon";

export default async function Home() {
  // Server-resolve auth so the CTA picks the right destination without a flash.
  const auth = await pylon.getAuth();

  return (
    <main className="mx-auto max-w-2xl px-6 py-20">
      <h1 className="text-5xl font-semibold tracking-tight">__APP_NAME__</h1>
      <p className="mt-3 text-lg text-muted-foreground">
        A Pylon app. Backend on <code className="font-mono text-sm">:4321</code>, Next.js on{" "}
        <code className="font-mono text-sm">:3000</code>.
      </p>

      <div className="mt-12 flex flex-wrap gap-3">
        <Button asChild size="lg">
          <Link href={auth ? "/dashboard" : "/login"}>
            {auth ? "Open dashboard" : "Sign in"}
            <ArrowRight className="ml-1" />
          </Link>
        </Button>
        <Button asChild variant="outline" size="lg">
          <a href="http://localhost:4321/studio" target="_blank" rel="noreferrer">
            Open Studio
            <ExternalLink className="ml-1" />
          </a>
        </Button>
      </div>

      <hr className="my-16 border-border" />

      <h2 className="text-xl font-semibold">What's wired up</h2>
      <ul className="mt-3 space-y-2 text-muted-foreground">
        <li className="flex gap-2">
          <span className="text-primary">→</span>
          <span>
            <strong className="text-foreground">Magic-code auth</strong> at{" "}
            <code className="font-mono text-sm">/login</code> via <code className="font-mono text-sm">@pylonsync/next</code> server actions.
          </span>
        </li>
        <li className="flex gap-2">
          <span className="text-primary">→</span>
          <span>
            <strong className="text-foreground">Cookie-gated dashboard</strong> at{" "}
            <code className="font-mono text-sm">/dashboard</code> with{" "}
            <code className="font-mono text-sm">proxy.ts</code> + server-side <code className="font-mono text-sm">requireAuth()</code>.
          </span>
        </li>
        <li className="flex gap-2">
          <span className="text-primary">→</span>
          <span>
            <strong className="text-foreground">Same-origin API proxy</strong> via{" "}
            <code className="font-mono text-sm">next.config.js</code> rewrites — no CORS needed.
          </span>
        </li>
        <li className="flex gap-2">
          <span className="text-primary">→</span>
          <span>
            <strong className="text-foreground">Tailwind 4 + shadcn</strong> with the components
            you'll actually use already in <code className="font-mono text-sm">components/ui/</code>.
          </span>
        </li>
      </ul>
    </main>
  );
}
