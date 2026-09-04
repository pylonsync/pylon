"use client";

import { Link } from "@pylonsync/react";
import { MarketingShell } from "./marketing-shell";

// The 404 body, for both audiences at once.
//
// A person needs a way back. An agent needs to know which URLs DO exist —
// this same markup is what the runtime converts when a markdown request hits a
// missing page, so the list below is the recovery path in both
// representations. Keep it short and keep every link real.

const DESTINATIONS: { href: string; label: string; note: string; external?: boolean }[] = [
	{ href: "/", label: "Home", note: "What Pylon is, in one page" },
	{ href: "/product", label: "Product", note: "Every framework primitive" },
	{ href: "/developers", label: "Developers", note: "Docs, CLI, MCP server, API spec" },
	{
		href: "/developers/examples",
		label: "Templates",
		note: "Eighteen working starter apps",
	},
	{
		href: "https://docs.pylonsync.com",
		label: "Documentation",
		note: "Concepts, auth, plugins, operations",
		external: true,
	},
];

const MACHINE_READABLE: { href: string; label: string; note: string }[] = [
	{ href: "/llms.txt", label: "/llms.txt", note: "What Pylon is for, and where everything lives" },
	{ href: "/sitemap.xml", label: "/sitemap.xml", note: "Every public page on this site" },
	{ href: "/openapi.json", label: "/openapi.json", note: "The agent API, described" },
	{ href: "/mcp.json", label: "/mcp.json", note: "The MCP server manifest" },
];

export function NotFoundView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<div className="mx-auto max-w-[760px] px-5 py-20 sm:px-8 sm:py-28">
				<p className="font-mono text-[12px] text-[var(--color-ink-4)]">404</p>
				<h1 className="mt-3 text-[32px] font-semibold leading-tight tracking-tight text-[var(--color-ink)] sm:text-[40px]">
					That page does not exist
				</h1>
				<p className="mt-5 max-w-[58ch] text-[15.5px] leading-[1.7] text-[var(--color-ink-2)]">
					The URL is wrong or the page has moved. Here is where to look next.
				</p>

				<h2 className="mt-12 text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
					Pages
				</h2>
				<ul className="mt-4 flex flex-col gap-2.5">
					{DESTINATIONS.map((d) => (
						<li key={d.href} className="text-[14.5px] leading-[1.5]">
							{d.external ? (
								<a
									href={d.href}
									className="text-[var(--color-ink)] underline underline-offset-4"
								>
									{d.label}
								</a>
							) : (
								<Link
									href={d.href}
									className="text-[var(--color-ink)] underline underline-offset-4"
								>
									{d.label}
								</Link>
							)}
							<span className="text-[var(--color-ink-3)]"> — {d.note}</span>
						</li>
					))}
				</ul>

				<h2 className="mt-10 text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
					Machine-readable
				</h2>
				<ul className="mt-4 flex flex-col gap-2.5">
					{MACHINE_READABLE.map((d) => (
						<li key={d.href} className="text-[14.5px] leading-[1.5]">
							<a
								href={d.href}
								className="font-mono text-[13.5px] text-[var(--color-ink)] underline underline-offset-4"
							>
								{d.label}
							</a>
							<span className="text-[var(--color-ink-3)]"> — {d.note}</span>
						</li>
					))}
				</ul>

				<p className="mt-10 text-[14px] leading-[1.7] text-[var(--color-ink-3)]">
					Still stuck?{" "}
					<Link
						href="/contact"
						className="text-[var(--color-ink)] underline underline-offset-4"
					>
						Contact us
					</Link>
					. If you followed a link from our own site, say where — that is a bug
					on our side.
				</p>
			</div>
		</MarketingShell>
	);
}
