"use client";

import { Link } from "@pylonsync/react";
import { ArrowUpRight, Check, Copy, Github } from "lucide-react";
import {
	createCommand,
	type Example,
	featuredExamples,
	templateRepoUrl,
} from "../lib/examples-content";
import { useCopy } from "../lib/use-copy";
import { FRAME_COL } from "./marketing-frame";

// Templates, directly under the hero bento. The reader has just seen what the
// framework does; this is the shortest path from that to a running app.
//
// Every card carries the three things someone needs to act: what it looks
// like, the command that scaffolds it, and the source to read first. The set
// is derived from the same examples data the /developers/examples page uses,
// so a template is described once.

const FEATURED = featuredExamples();

export function TemplatesStrip() {
	return (
		<section
			id="templates"
			className="border-t border-[var(--color-rule)] bg-[var(--color-paper-1)]"
		>
			<div className={`${FRAME_COL} px-5 py-16 sm:px-8 sm:py-20 lg:py-24`}>
				<div className="flex flex-col gap-5 sm:flex-row sm:items-end sm:justify-between">
					<div>
						<h2 className="max-w-[20ch] text-[clamp(30px,4vw,46px)] font-semibold leading-[1.05] tracking-[-0.035em] text-[var(--color-ink)]">
							Start from a working app.
						</h2>
						<p className="mt-5 max-w-[620px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
							Every template is a complete Pylon app with a schema, access
							rules, server functions, and server-rendered pages. Run the
							command, read the source, change what you need.
						</p>
					</div>
					<Link
						href="/developers/examples"
						className="group inline-flex shrink-0 items-center gap-1.5 text-[14px] font-medium text-[var(--color-brand)] hover:underline"
					>
						All templates
						<ArrowUpRight className="size-3.5 transition-transform duration-200 group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
					</Link>
				</div>

				<div className="mt-12 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
					{FEATURED.map((ex) => (
						<TemplateCard key={ex.template} ex={ex} />
					))}
				</div>
			</div>
		</section>
	);
}

function TemplateCard({ ex }: { ex: Example }) {
	const { copied, copy } = useCopy();
	// FEATURED is filtered to examples that have a template, so this is always
	// set; the fallback keeps the card renderable if the featured list is ever
	// pointed at a live-demo-only entry.
	const template = ex.template ?? "";
	const command = createCommand(template);

	return (
		<div className="group flex flex-col overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)] transition-[border-color,box-shadow] duration-200 ease-[var(--ease-out-quart)] hover:border-[var(--color-ink-4)] hover:shadow-[var(--shadow-card-hover)]">
			<div className="relative aspect-[16/10] overflow-hidden border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				{ex.shot ? (
					<img
						src={ex.shot}
						alt={`The ${ex.name} template running`}
						loading="lazy"
						decoding="async"
						className="size-full object-cover object-top"
					/>
				) : (
					<ShotPlaceholder name={ex.name} />
				)}
				{ex.live ? (
					<span className="absolute left-3 top-3 inline-flex items-center gap-1.5 rounded-full bg-[var(--color-paper)]/92 px-2.5 py-1 font-mono text-[10px] uppercase tracking-[0.08em] text-[var(--color-status-live)] shadow-[var(--shadow-card)]">
						<span className="block size-1.5 rounded-full bg-[var(--color-status-live)]" />
						Live demo
					</span>
				) : null}
			</div>

			<div className="flex flex-1 flex-col gap-3 p-5">
				<div>
					<h3 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
						{ex.name}
					</h3>
					<p className="mt-1.5 text-[13px] leading-[1.55] text-[var(--color-ink-3)]">
						{ex.blurb}
					</p>
				</div>

				<button
					type="button"
					onClick={() => copy(command)}
					aria-label={`Copy: ${command}`}
					title={command}
					className="mt-auto flex w-full items-center gap-2 rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-2.5 py-2 text-left font-mono text-[11px] text-[var(--color-ink-2)] transition-colors hover:border-[var(--color-brand)]/50 hover:bg-[var(--color-paper)]"
				>
					<span className="select-none text-[var(--color-brand)]">$</span>
					{/* The command is long and the card is narrow, so the pill shows the
					    part that varies — the template name — and copies the whole
					    thing. The full command is in the title + aria-label. */}
					<span className="truncate">
						create …{" "}
						<span className="text-[var(--color-ink)]">
							--template {template}
						</span>
					</span>
					<span className="ml-auto shrink-0 text-[var(--color-ink-4)]">
						{copied ? (
							<Check className="size-3.5 text-[var(--color-status-live)]" />
						) : (
							<Copy className="size-3.5" />
						)}
					</span>
				</button>

				<div className="flex items-center gap-3 text-[12px]">
					<a
						href={templateRepoUrl(template)}
						className="inline-flex items-center gap-1.5 text-[var(--color-ink-3)] transition-colors hover:text-[var(--color-ink)]"
					>
						<Github className="size-3.5" />
						Source
					</a>
					{ex.live ? (
						<a
							href={ex.live}
							className="inline-flex items-center gap-1 text-[var(--color-brand)] hover:underline"
						>
							Open demo
							<ArrowUpRight className="size-3" />
						</a>
					) : null}
				</div>
			</div>
		</div>
	);
}

/**
 * Drawn stand-in for a template with no capture yet.
 *
 * A designed placeholder rather than an <img> with an onError swap: the page
 * is server-rendered, so a missing file would paint a broken image before
 * hydration could replace it. Whether a shot exists is data, not a runtime
 * error.
 */
function ShotPlaceholder({ name }: { name: string }) {
	return (
		<div
			aria-hidden="true"
			className="flex size-full flex-col gap-2 bg-[var(--color-paper-1)] p-4"
		>
			<div className="flex items-center gap-1.5">
				<span className="block size-1.5 rounded-full bg-[var(--color-rule)]" />
				<span className="block size-1.5 rounded-full bg-[var(--color-rule)]" />
				<span className="block size-1.5 rounded-full bg-[var(--color-rule)]" />
				<span className="ml-2 h-1.5 w-20 rounded-full bg-[var(--color-rule)]" />
			</div>
			<div className="flex flex-1 items-center justify-center">
				<span className="font-mono text-[11px] text-[var(--color-ink-4)]">
					{name}
				</span>
			</div>
			<div className="flex gap-1.5">
				<span className="h-6 flex-1 rounded-[3px] bg-[var(--color-rule)]/60" />
				<span className="h-6 flex-1 rounded-[3px] bg-[var(--color-rule)]/40" />
				<span className="h-6 flex-1 rounded-[3px] bg-[var(--color-rule)]/25" />
			</div>
		</div>
	);
}
