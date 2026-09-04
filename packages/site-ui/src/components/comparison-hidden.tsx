import { Link } from "@pylonsync/react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";

// Rendered in place of the /vs comparison pages while they're hidden
// pre-launch (COMPARISONS_ENABLED === false in lib/comparison-content.ts). The
// routes also set a 404 status and noindex these URLs so they drop out of
// search; this body just keeps the marketing chrome intact for anyone who
// follows a stale link.
export function ComparisonHidden({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<div className="mx-auto max-w-[640px] px-5 py-32 text-center">
				<h1 className="text-[28px] font-semibold tracking-tight">
					Page not found
				</h1>
				<p className="mt-3 text-[var(--color-ink-3)]">
					This page isn&apos;t available.
				</p>
				<Button asChild variant="primary" size="lg" className="mt-8">
					<Link href="/product">Explore what Pylon offers →</Link>
				</Button>
			</div>
		</MarketingShell>
	);
}
