// app/rate-limit.tsx — the 429 page shown when a browser navigation trips the
// per-IP rate limiter. Pylon pre-renders this to static HTML at build (the
// rate limiter short-circuits before SSR, so it can't render per request) and
// serves it in place of the framework default. Keep it self-contained: it's
// rendered with no props and no client JS, so no hooks / data fetching here.
export default function RateLimit() {
  return (
    <main className="grid min-h-screen place-items-center bg-background px-6 text-center">
      <div className="flex max-w-md flex-col items-center gap-4">
        <div className="text-6xl font-bold tracking-tight text-primary">429</div>
        <h1 className="text-2xl font-semibold text-foreground">
          Whoa — slow down a sec
        </h1>
        <p className="text-sm text-muted-foreground">
          You&rsquo;re browsing faster than our little demo shop can restock the
          shelves. Give it a few seconds and refresh — your cart is safe.
        </p>
        <a
          href="/"
          className="mt-2 rounded-md border border-input px-5 py-2 text-sm font-medium text-foreground transition-colors hover:bg-accent"
        >
          Back to the catalog
        </a>
      </div>
    </main>
  );
}
