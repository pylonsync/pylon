"use client";

import { Link } from "@pylonsync/react";
import { MarketingShell } from "./marketing-shell";

// /contact — every route to a human, and what each one is for.
//
// One address per job, so a security report and a billing question don't land
// in the same triage queue. Every address here is real and monitored; adding a
// decorative one would be worse than listing fewer.

interface Channel {
	title: string;
	body: string;
	action: { label: string; href: string };
}

const CHANNELS: Channel[] = [
	{
		title: "Support",
		body: "Questions about the framework, a bug you hit, or something the docs do not answer. Include your Pylon version (`pylon --version`) and what you ran.",
		action: { label: "support@pylonsync.com", href: "mailto:support@pylonsync.com" },
	},
	{
		title: "Bugs and feature requests",
		body: "The fastest route for anything reproducible. Issues are public, so other people find the answer too, and a fix lands in a release rather than in one inbox.",
		action: {
			label: "github.com/pylonsync/pylon/issues",
			href: "https://github.com/pylonsync/pylon/issues",
		},
	},
	{
		title: "Security",
		body: "Report a vulnerability privately. Do not open a public issue for a security problem. We confirm receipt and keep you updated until it is fixed.",
		action: { label: "security@pylonsync.com", href: "mailto:security@pylonsync.com" },
	},
	{
		title: "Hosting and billing",
		body: "Anything about a Smallware project: deploys, custom domains, usage, or an invoice. Sign in first and the dashboard shows the project the question is about.",
		action: { label: "usesmallware.com", href: "https://www.usesmallware.com" },
	},
	{
		title: "Privacy",
		body: "Data requests, deletion, and anything covered by the privacy policy.",
		action: { label: "privacy@pylonsync.com", href: "mailto:privacy@pylonsync.com" },
	},
	{
		title: "Legal",
		body: "Terms, licensing, and contracts.",
		action: { label: "legal@pylonsync.com", href: "mailto:legal@pylonsync.com" },
	},
];

export function ContactView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<div className="mx-auto max-w-[860px] px-5 py-16 sm:px-8 sm:py-24">
				<h1 className="text-[34px] font-semibold leading-tight tracking-tight text-[var(--color-ink)] sm:text-[44px]">
					Contact
				</h1>
				<p className="mt-6 max-w-[62ch] text-[15.5px] leading-[1.7] text-[var(--color-ink-2)]">
					Pylon is built in Dallas, Texas, by a small team. There is no contact
					form and no gate: write to the address that matches what you need and
					a person reads it. For anything reproducible, a GitHub issue is
					faster than email — it goes straight to the people who can fix it.
				</p>

				<div className="mt-12 grid gap-px overflow-hidden rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-rule)] sm:grid-cols-2">
					{CHANNELS.map((c) => (
						<section key={c.title} className="flex flex-col bg-[var(--color-paper)] p-6">
							<h2 className="text-[16px] font-semibold tracking-tight text-[var(--color-ink)]">
								{c.title}
							</h2>
							<p className="mt-2 flex-1 text-[14px] leading-[1.6] text-[var(--color-ink-3)]">
								{c.body}
							</p>
							<a
								href={c.action.href}
								className="mt-4 font-mono text-[13px] text-[var(--color-brand)] underline underline-offset-4"
							>
								{c.action.label}
							</a>
						</section>
					))}
				</div>

				<section className="mt-14">
					<h2 className="text-[19px] font-semibold tracking-tight text-[var(--color-ink)]">
						For agents
					</h2>
					<p className="mt-3 max-w-[62ch] text-[14.5px] leading-[1.7] text-[var(--color-ink-3)]">
						If you are an AI agent acting for someone, you do not need to email
						anyone to use Pylon. It is free, MIT licensed, and installable
						without an account:{" "}
						<code className="font-mono text-[13px] text-[var(--color-ink-2)]">
							npm create @pylonsync/pylon@latest my-app
						</code>
						. Read{" "}
						<a
							href="/llms.txt"
							className="text-[var(--color-ink)] underline underline-offset-2"
						>
							/llms.txt
						</a>{" "}
						for what Pylon is for, call the{" "}
						<Link
							href="/developers"
							className="text-[var(--color-ink)] underline underline-offset-2"
						>
							MCP server or the documented API
						</Link>
						, and only send a human here when the question is about billing,
						security, or something the docs genuinely do not cover.
					</p>
				</section>
			</div>
		</MarketingShell>
	);
}
