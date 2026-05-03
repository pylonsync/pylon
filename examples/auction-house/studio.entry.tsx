/**
 * Custom Studio extensions for the Auction House example.
 *
 * This file is bundled by the Pylon CLI (`bun build`) into a single
 * ESM module served at `/studio/extensions.js`. The Studio shell
 * dynamic-imports it at boot when `studio.config.ts.hasExtensions`
 * is true.
 *
 * `react`, `react-dom`, and `react/jsx-runtime` are externalized — the
 * Studio host provides them via an import map, so this bundle stays
 * tiny.
 */
import { defineStudioExtensions } from "@pylonsync/sdk";
import type {
  StudioCellRendererProps,
  StudioPageProps,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Custom column renderer: paddle number derived from auction + lot id.
// ---------------------------------------------------------------------------

function PaddleCell({ value, row }: StudioCellRendererProps) {
  const auction = String(value ?? "");
  const lot = String(row.id ?? "");
  const paddle = (hash(`${auction}-${lot}`) % 9000 + 1000).toString();
  return (
    <span className="inline-flex items-center gap-1.5 rounded-md bg-status-amber-bg px-2 py-0.5 text-xs font-mono text-status-amber-fg">
      #{paddle}
    </span>
  );
}

function hash(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = (h * 33) ^ s.charCodeAt(i);
  }
  return Math.abs(h | 0);
}

// ---------------------------------------------------------------------------
// Custom page: streaming bid feed.
// ---------------------------------------------------------------------------

function LiveBidsPage({ api }: StudioPageProps) {
  // Wired up minimally — the example demonstrates how a page can use
  // the Pylon `api()` helper. A real implementation would subscribe
  // via /api/sync/* for true streaming.
  const fetchRecent = async () => {
    return api<{ data: unknown[] }>("/api/entities/Bid?page=1&per_page=20&sort=createdAt&order=desc");
  };

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Live bid feed</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Custom dashboard powered by studio.entry.tsx — call `api()` from
          your component to read the live state.
        </p>
      </div>
      <button
        type="button"
        className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90"
        onClick={() => {
          fetchRecent().then((r) => console.log("[bid feed]", r));
        }}
      >
        Fetch latest 20 bids (check console)
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

export default defineStudioExtensions({
  renderers: {
    paddle: PaddleCell,
  },
  pages: {
    liveBids: LiveBidsPage,
  },
});
