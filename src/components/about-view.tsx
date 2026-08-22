"use client";

import { Link } from "@pylonsync/react";
import { MarketingShell } from "./marketing-shell";

// /about — what Pylon is, who builds it, and how it is paid for.
//
// This page exists for people deciding whether to depend on us, and for the
// agents that answer them. So it says the checkable things: the licence, where
// the source is, what the hosted product is, and how to reach a human. No
// claims a reader cannot verify from the repo.

const FACTS: { label: string; value: React.ReactNode }[] = [
	{ label: "What it is", value: "A full-stack web framework" },
	{ label: "Licence", value: "MIT" },
	{
		label: "Source",
		value: (
			<a href="https://github.com/pylonsync/pylon">github.com/pylonsync/pylon</a>
		),
	},
	{ label: "Runtime", value: "A Rust server that runs your TypeScript on Bun" },
	{ label: "Database", value: "SQLite by default, Postgres when you need it" },
	{ label: "Built in", value: "Dallas, Texas" },
];

export function AboutView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<div className="mx-auto max-w-[760px] px-5 py-16 sm:px-8 sm:py-24">
				<h1 className="text-[34px] font-semibold leading-tight tracking-tight text-[var(--color-ink)] sm:text-[44px]">
					About Pylon
				</h1>

				<div className="mt-8 flex flex-col gap-6 text-[15.5px] leading-[1.7] text-[var(--color-ink-2)] [&_a]:text-[var(--color-ink)] [&_a]:underline [&_a]:underline-offset-2">
					<p>
						Pylon is a full-stack framework for building real applications. One
						binary holds the pieces a product normally assembles from four or
						five services: a typed schema with automatic migrations, row-level
						access policies, server functions, live queries over WebSocket,
						authentication, and React server rendering. It runs on SQLite by
						default and on Postgres when a project outgrows one machine.
					</p>
					<p>
						It is built for coding agents as much as for people. An agent that
						writes an app has to hold the whole stack in its head at once, and
						every extra service is another chance to get the wiring wrong.
						Pylon collapses that: the schema, the access rules, the server
						code, and the frontend live in one project, in one language, behind
						one command. The framework ships an agent skill file, an MCP
						server, and machine-readable docs so an agent can check its own
						work instead of guessing.
					</p>
					<p>
						The framework is free and open source under the MIT licence. The
						source is on{" "}
						<a href="https://github.com/pylonsync/pylon">GitHub</a>, including
						the Rust runtime, the TypeScript SDKs, eighteen starter templates,
						and the example apps. Anyone can read it, fork it, self-host it, or
						run it with no account at all.
					</p>
					<p>
						The work is paid for by{" "}
						<a href="https://www.usesmallware.com">Smallware</a>, managed
						hosting for Pylon apps. Smallware runs the same binary you run
						locally, deploys with one command, and charges by usage with a free
						tier and no monthly minimum. Hosting is the product; the framework
						is not a trial of it. Nothing in Pylon is gated behind an account,
						and nothing about self-hosting is second-class — the deploy targets
						for Fly, Docker, Compose, and a plain systemd unit are maintained
						in the same repo.
					</p>
					<p>
						Pylon is built in Dallas, Texas. It is early software under active
						development: releases are frequent, the changelog is public, and
						the fastest way to change its direction is an issue or a pull
						request on the repo. If you are evaluating it for something that
						matters, read the{" "}
						<a href="https://docs.pylonsync.com/introduction">docs</a>, run{" "}
						<code className="font-mono text-[13.5px]">
							npm create @pylonsync/pylon@latest
						</code>
						, and tell us where it falls short.
					</p>
				</div>

				<dl className="mt-12 grid grid-cols-1 gap-px overflow-hidden rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-rule)] sm:grid-cols-2">
					{FACTS.map((f) => (
						<div key={f.label} className="bg-[var(--color-paper)] p-5">
							<dt className="font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
								{f.label}
							</dt>
							<dd className="mt-2 text-[14.5px] leading-[1.5] text-[var(--color-ink-2)] [&_a]:text-[var(--color-ink)] [&_a]:underline [&_a]:underline-offset-2">
								{f.value}
							</dd>
						</div>
					))}
				</dl>

				<div className="mt-10 flex flex-wrap gap-x-6 gap-y-2 text-[14px]">
					<Link
						href="/contact"
						className="text-[var(--color-brand)] underline underline-offset-4"
					>
						Contact us
					</Link>
					<Link
						href="/developers"
						className="text-[var(--color-brand)] underline underline-offset-4"
					>
						Developer resources
					</Link>
					<a
						href="https://docs.pylonsync.com"
						className="text-[var(--color-brand)] underline underline-offset-4"
					>
						Documentation
					</a>
				</div>
			</div>
		</MarketingShell>
	);
}
