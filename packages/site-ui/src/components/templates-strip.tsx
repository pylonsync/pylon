"use client";

import { Image, Link } from "@pylonsync/react";
import { useEffect, useState } from "react";
import { Check, Copy, Github } from "lucide-react";
import {
	createCommand,
	type Example,
	featuredExamples,
	templateRepoUrl,
} from "../lib/examples-content";
import { useCopy } from "../lib/use-copy";
import { FRAME_COL } from "./marketing-frame";
import { TransitionChevron } from "./transition-chevron";

const FEATURED = featuredExamples();

export function TemplatesStrip() {
	const [activeIndex, setActiveIndex] = useState(0);
	const active = FEATURED[activeIndex] ?? FEATURED[0];

	if (!active) return null;

	return (
		<section
			id="templates"
			className="border-t border-[var(--color-rule)] bg-[var(--color-paper-1)]"
		>
			<div className={FRAME_COL}>
				<div className="border-b border-[var(--color-rule)] px-5 py-3 sm:px-8">
					<span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						Templates
					</span>
				</div>
				<div className="flex flex-col gap-5 border-b border-[var(--color-rule)] px-5 py-16 sm:flex-row sm:items-end sm:justify-between sm:px-8 sm:py-20 lg:py-24">
					<div>
						<h2 className="max-w-[20ch] text-[clamp(30px,4vw,46px)] font-semibold leading-[1.05] tracking-[-0.035em] text-[var(--color-ink)]">
							Start from a working app.
						</h2>
						<p className="mt-5 max-w-[620px] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
							Choose a complete app. Run one command, read the source, and
							change what you need.
						</p>
					</div>
					<Link
						href="/developers/examples"
						className="t-learn inline-flex shrink-0 items-center gap-1.5 text-[14px] font-medium text-[var(--color-brand)] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] focus-visible:ring-offset-2"
					>
						All templates
						<TransitionChevron />
					</Link>
				</div>

				<div className="bg-[var(--color-paper)] lg:grid lg:grid-cols-[280px_minmax(0,1fr)]">
					<div
						className="flex overflow-x-auto border-b border-[var(--color-rule)] lg:flex-col lg:overflow-visible lg:border-b-0 lg:border-r lg:border-[var(--color-rule)]"
						role="group"
						aria-label="Featured templates"
					>
						{FEATURED.map((example, index) => {
							const selected = index === activeIndex;
							return (
								<button
									key={example.template}
									type="button"
									aria-pressed={selected}
									onClick={() => setActiveIndex(index)}
									className={`min-w-[72%] border-r border-[var(--color-rule)] px-5 py-4 text-left transition-[background-color,color,box-shadow] duration-200 last:border-r-0 focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--color-brand)] sm:min-w-[42%] lg:min-w-0 lg:border-b lg:border-r-0 lg:last:border-b-0 ${
										selected
											? "bg-[var(--color-paper-2)] text-[var(--color-ink)] shadow-[inset_2px_0_0_var(--color-ink)]"
											: "text-[var(--color-ink-3)] hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)]"
									}`}
								>
									<span className="block text-[14px] font-semibold">
										{example.name}
									</span>
									<span className="mt-1 block truncate font-mono text-[10.5px] text-[var(--color-ink-4)]">
										{example.shows.slice(0, 2).join(" + ")}
									</span>
								</button>
							);
						})}
					</div>

					<TemplateDetail key={active.template} example={active} />
				</div>
			</div>
		</section>
	);
}

function TemplateDetail({ example }: { example: Example }) {
	const { copied, copy } = useCopy();
	const [shown, setShown] = useState(false);
	const template = example.template ?? "";
	const command = createCommand(template);

	useEffect(() => {
		const frame = requestAnimationFrame(() => setShown(true));
		return () => cancelAnimationFrame(frame);
	}, []);

	return (
		<div
			className="template-detail-panel t-panel-slide min-w-0 overflow-hidden"
			data-open={shown}
		>
			<div className="aspect-[16/9] overflow-hidden bg-[var(--color-paper-2)]">
				{example.shot ? (
					<Image
						src={example.shot}
						alt={`${example.name} template home page`}
						width={1440}
						height={810}
						sizes="(min-width: 1024px) 900px, 100vw"
						widths={[640, 828, 1200]}
						className="block size-full object-cover object-top"
					/>
				) : (
					<div className="flex size-full items-center justify-center px-6 text-center font-mono text-[12px] text-[var(--color-ink-4)]">
						Preview not available
					</div>
				)}
			</div>

			<div>
				<div className="grid min-w-0 gap-7 border-t border-[var(--color-rule)] p-6 sm:p-8 md:grid-cols-[minmax(0,1fr)_minmax(280px,0.8fr)] md:items-end">
					<div className="min-w-0">
						<h3 className="text-[24px] font-semibold leading-tight tracking-[-0.025em] text-[var(--color-ink)]">
							{example.name}
						</h3>
						<p className="mt-3 max-w-[620px] text-[15px] leading-[1.65] text-[var(--color-ink-2)]">
							{example.blurb}
						</p>
					</div>

					<div className="grid min-w-0 gap-3">
						<button
							type="button"
							onClick={() => copy(command)}
							aria-label={`Copy: ${command}`}
							className="flex w-full items-center gap-2 rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-2.5 text-left font-mono text-[11px] text-[var(--color-ink-2)] transition-colors hover:border-[var(--color-brand)]/50 hover:bg-[var(--color-paper)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)]"
						>
							<span className="select-none text-[var(--color-brand)]">$</span>
							<span className="min-w-0 flex-1 truncate">{command}</span>
							<span className="ml-auto shrink-0 text-[var(--color-ink-4)]">
								{copied ? (
									<Check className="size-3.5 text-[var(--color-status-live)]" />
								) : (
									<Copy className="size-3.5" />
								)}
							</span>
						</button>

						<div className="flex items-center gap-4 text-[12.5px]">
							<a
								href={templateRepoUrl(template)}
								className="inline-flex items-center gap-1.5 text-[var(--color-ink-3)] transition-colors hover:text-[var(--color-ink)]"
							>
								<Github className="size-3.5" />
								Source
							</a>
							{example.live ? (
								<a
									href={example.live}
									className="t-learn inline-flex items-center gap-1 text-[var(--color-brand)] hover:underline"
								>
									Open demo
									<TransitionChevron />
								</a>
							) : null}
						</div>
					</div>
				</div>
			</div>
		</div>
	);
}
