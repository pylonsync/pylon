// Marketing-site chrome (nav + footer) reused by every page that
// isn't the giant `/` landing page. Wraps children in the
// `.pylon-landing` class so the existing design tokens + buttons
// + container styles apply.
//
// The landing page itself does NOT use this — it ships the chrome
// inline with the giant DESIGN_CSS string. That's deliberate: the
// landing page has a unique hero layout we don't want to share by
// accident. Future split: lift DESIGN_CSS into a separate file and
// have both the landing page + this shell import it.

import Link from "next/link";
import { PylonMark } from "@/components/pylon-logo";
import { MARKETING_SHELL_CSS } from "@/lib/marketing-shell-css";

export interface MarketingShellProps {
	children: React.ReactNode;
	/**
	 * GitHub star count rendered next to the GitHub link in the
	 * nav. Server-rendered at build time so there's no client
	 * fetch on page load. Pass null when unavailable (build had no
	 * network); the nav collapses to "GitHub" plain text.
	 */
	stars: number | null;
}

export function MarketingShell({ children, stars }: MarketingShellProps) {
	return (
		<div className="pylon-landing">
			<style dangerouslySetInnerHTML={{ __html: MARKETING_SHELL_CSS }} />
			<MarketingNav stars={stars} />
			{children}
			<MarketingFooter />
		</div>
	);
}

function MarketingNav({ stars }: { stars: number | null }) {
	return (
		<nav className="nav">
			<div className="shell nav-inner">
				<Link className="brand" href="/">
					<PylonMark size={20} style={{ color: "var(--ink)" }} />
					Pylon
				</Link>
				<ul className="nav-links">
					<li>
						<Link href="/#features">Product</Link>
					</li>
					<li>
						<a href="https://docs.pylonsync.com">Docs</a>
					</li>
					<li>
						<a href="https://github.com/pylonsync/pylon/releases">Changelog</a>
					</li>
					<li>
						<Link href="/vs">Compare</Link>
					</li>
				</ul>
				<div className="nav-cta">
					<a
						className="nav-github"
						href="https://github.com/pylonsync/pylon"
						target="_blank"
						rel="noopener noreferrer"
						aria-label="Pylon on GitHub"
					>
						<svg
							width="14"
							height="14"
							viewBox="0 0 16 16"
							fill="currentColor"
							aria-hidden="true"
						>
							<path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
						</svg>
						{stars !== null ? (
							<span className="nav-github-stars">★ {formatStars(stars)}</span>
						) : (
							<span className="nav-github-stars">GitHub</span>
						)}
					</a>
					<Link className="btn ghost" href="https://cloud.pylonsync.com/login">
						Sign in
					</Link>
					<Link
						className="btn dark"
						href="https://cloud.pylonsync.com/signup"
					>
						Start building →
					</Link>
				</div>
			</div>
		</nav>
	);
}

function MarketingFooter() {
	return (
		<footer className="footer">
			<div className="shell footer-inner">
				<div className="footer-brand">
					<PylonMark size={20} style={{ color: "var(--ink)" }} />
					<span>Pylon</span>
					<span className="footer-tag">Realtime backend for TypeScript apps.</span>
				</div>
				<div className="footer-cols">
					<div className="footer-col">
						<h6>Product</h6>
						<Link href="/">Overview</Link>
						<a href="https://docs.pylonsync.com">Docs</a>
						<a href="https://github.com/pylonsync/pylon/releases">Changelog</a>
						<Link href="/skill">Claude Code skill</Link>
					</div>
					<div className="footer-col">
						<h6>Compare</h6>
						<Link href="/vs/convex">vs. Convex</Link>
						<Link href="/vs/supabase">vs. Supabase</Link>
						<Link href="/vs/firebase">vs. Firebase</Link>
						<Link href="/vs/colyseus">vs. Colyseus</Link>
						<Link href="/vs/playroom">vs. Playroom Kit</Link>
						<Link href="/vs/nakama">vs. Nakama</Link>
					</div>
					<div className="footer-col">
						<h6>Cloud</h6>
						<a href="https://cloud.pylonsync.com/signup">Sign up</a>
						<a href="https://cloud.pylonsync.com/login">Sign in</a>
						<a href="https://docs.pylonsync.com/cloud">Pylon Cloud</a>
						<a href="https://status.pylonsync.com">Status</a>
					</div>
					<div className="footer-col">
						<h6>Code</h6>
						<a href="https://github.com/pylonsync/pylon">GitHub</a>
						<a href="https://github.com/pylonsync/pylon/issues">Issues</a>
						<a href="https://github.com/pylonsync/pylon/discussions">Discussions</a>
						<a href="https://twitter.com/pylonsync">Twitter</a>
					</div>
				</div>
				<div className="footer-base">
					<span>© Pylon. MIT / Apache-2.0.</span>
					<span>Built in Dallas.</span>
				</div>
			</div>
		</footer>
	);
}

function formatStars(n: number): string {
	if (n < 1000) return String(n);
	if (n < 10_000) return `${(n / 1000).toFixed(1)}k`;
	return `${Math.round(n / 1000)}k`;
}
