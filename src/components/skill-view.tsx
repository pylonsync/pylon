"use client";

import { useState } from "react";
import { Link } from "@pylonsync/react";
import { Check, Copy } from "lucide-react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";

const INSTALL_CMD = "npx skills add pylonsync/pylon";

export function SkillView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[820px] px-5 pb-14 pt-20 sm:px-8 sm:pb-16 sm:pt-28">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-brand)]">
						For Claude Code
					</div>
					<h1 className="mt-4 max-w-[16ch] text-[clamp(34px,5vw,58px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						Teach Claude to write Pylon.
					</h1>
					<p className="mt-6 max-w-[58ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						One file teaches Claude Code Pylon&rsquo;s schema, row-level policies,
						server functions, React client, and deployment workflow. Drop it in
						and Claude writes Pylon that compiles. The skill ships with the
						framework, so it stays current.
					</p>
				</div>
			</header>

			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[820px] px-5 py-14 sm:px-8 sm:py-16">
					<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
						Install
					</h2>
					<p className="mt-3 text-[14px] leading-[1.6] text-[var(--color-ink-2)]">
						Run one command, or copy the file by hand into{" "}
						<code className="rounded-[2px] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-1.5 py-0.5 font-mono text-[12.5px] text-[var(--color-ink)]">
							~/.claude/skills/pylon/SKILL.md
						</code>{" "}
						(user-wide) or{" "}
						<code className="rounded-[2px] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-1.5 py-0.5 font-mono text-[12.5px] text-[var(--color-ink)]">
							.claude/skills/pylon/SKILL.md
						</code>{" "}
						in your repo. Restart Claude Code and it loads automatically.
					</p>

					<div className="mt-6">
						<CopyBlock command={INSTALL_CMD} />
					</div>

					<div className="mt-6 flex flex-col items-start gap-3 sm:flex-row sm:items-center">
						<Button asChild variant="default" size="sm">
							<a href="/pylon-skill.md">View the raw skill →</a>
						</Button>
						<span className="text-[12.5px] text-[var(--color-ink-4)]">
							Served at{" "}
							<code className="font-mono">pylonsync.com/pylon-skill.md</code>
						</span>
					</div>
				</div>
			</section>

			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[820px] px-5 py-14 sm:px-8 sm:py-16">
					<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
						What it teaches
					</h2>
					<ul className="mt-6 grid gap-x-8 gap-y-3 sm:grid-cols-2">
						{[
							"The schema model: entity(), field.*, indexes, migrations",
							"Row-level policies and the access-rule expression language",
							"Server functions: query / mutation / action with validators",
							"The React client: db.useQuery, rooms, file uploads",
							"Auth: magic-link, OAuth, OIDC, guest sessions",
							"Deployment: pylon dev locally, Smallware in production",
						].map((item) => (
							<li key={item} className="flex items-start gap-3">
								<span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-[var(--color-brand)]/12 text-[var(--color-brand)]">
									<Check className="size-3" />
								</span>
								<span className="text-[14px] leading-[1.5] text-[var(--color-ink-2)]">
									{item}
								</span>
							</li>
						))}
					</ul>
				</div>
			</section>

			<section>
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[18ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Then build something.
					</h2>
					<div className="mt-7 inline-flex items-center gap-3 rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2.5 font-mono text-[13.5px] text-[var(--color-ink)]">
						<span className="text-[var(--color-brand)]">$</span>
						npm create @pylonsync/pylon@latest
					</div>
					<div className="mt-8">
						<Button asChild variant="ghost" size="lg">
							<Link href="/developers/examples">See example apps →</Link>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}

// A copyable command block; click to copy the whole command.
function CopyBlock({ command }: { command: string }) {
	const [copied, setCopied] = useState(false);
	function copy() {
		navigator.clipboard
			?.writeText(command)
			.then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), 1600);
			})
			.catch(() => {});
	}
	return (
		<button
			type="button"
			onClick={copy}
			aria-label="Copy install command"
			className="group flex w-full items-start gap-3 overflow-x-auto rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[#0c0d10] px-4 py-3 text-left font-mono text-[12.5px] leading-[1.6] text-[var(--color-ink)] transition-colors hover:border-[var(--color-brand)]/50"
		>
			<span className="select-none text-[var(--color-brand)]">$</span>
			<span className="flex-1 whitespace-pre-wrap break-all">{command}</span>
			<span className="shrink-0 text-[var(--color-ink-3)] transition-colors group-hover:text-[var(--color-ink)]">
				{copied ? (
					<Check className="size-4 text-[var(--color-status-live)]" />
				) : (
					<Copy className="size-4" />
				)}
			</span>
		</button>
	);
}
