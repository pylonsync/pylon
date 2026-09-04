import type { ReactNode } from "react";
import { MarketingShell } from "./marketing-shell";

// Shared chrome for the /privacy and /terms pages: the marketing shell (nav +
// footer) wrapped around a narrow, readable prose column. Static content only —
// no hooks, no client state — so it renders straight through SSR.
export function LegalPage({
	title,
	lastUpdated,
	intro,
	signedIn = false,
	children,
}: {
	title: string;
	lastUpdated: string;
	intro: string;
	signedIn?: boolean;
	children: ReactNode;
}) {
	return (
		<MarketingShell signedIn={signedIn}>
			<div className="mx-auto max-w-[760px] px-5 py-16 sm:px-8 sm:py-24">
				<p className="font-mono text-[11px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
					Legal
				</p>
				<h1 className="mt-3 text-[32px] font-semibold leading-tight tracking-tight text-[var(--color-ink)] sm:text-[40px]">
					{title}
				</h1>
				<p className="mt-3 font-mono text-[12px] text-[var(--color-ink-4)]">
					Last updated {lastUpdated}
				</p>
				<p className="mt-7 text-[15.5px] leading-[1.65] text-[var(--color-ink-2)]">
					{intro}
				</p>
				<div className="mt-12 flex flex-col gap-10">{children}</div>
			</div>
		</MarketingShell>
	);
}

export function LegalSection({
	heading,
	children,
}: {
	heading: string;
	children: ReactNode;
}) {
	return (
		<section className="flex flex-col gap-3">
			<h2 className="text-[19px] font-semibold tracking-tight text-[var(--color-ink)]">
				{heading}
			</h2>
			<div className="flex flex-col gap-3 text-[14.5px] leading-[1.7] text-[var(--color-ink-3)] [&_a]:text-[var(--color-ink)] [&_a]:underline [&_a]:underline-offset-2 [&_strong]:font-semibold [&_strong]:text-[var(--color-ink-2)]">
				{children}
			</div>
		</section>
	);
}

// A tidy bulleted list for the dense sections (data collected, acceptable use…).
export function LegalList({ items }: { items: ReactNode[] }) {
	return (
		<ul className="flex list-disc flex-col gap-1.5 pl-5 marker:text-[var(--color-ink-4)]">
			{items.map((item, i) => (
				<li key={i}>{item}</li>
			))}
		</ul>
	);
}
