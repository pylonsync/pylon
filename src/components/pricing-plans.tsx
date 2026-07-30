import type { ReactNode } from "react";
import { Link } from "@pylonsync/react";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";

// The three Pylon Cloud plans — shared by the homepage pricing section and the
// dedicated /pricing page so the numbers live in exactly one place. Plain
// component (no client directive); rendered inside client trees on both pages.

export function PricingPlans({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<>
			<div className="grid gap-4 md:grid-cols-3">
				<PlanCard
					name="Hobby"
					price="$0"
					per="forever"
					blurb="One project for weekend builds and your first users."
					includes={[
						"1 project, 1 organization",
						"Shared 1 GB RAM · 3 GB volume",
						"100k requests / month",
						"Single region, autostop on idle",
						"SQLite, magic-link auth",
					]}
					cta={
						<Button asChild variant="default" size="full">
							<Link href={signedIn ? "/dashboard" : "/signup"}>
								{signedIn ? "Open dashboard" : "Start free"}
							</Link>
						</Button>
					}
				/>
				<PlanCard
					featured
					name="Pro"
					price="$25"
					per="org / month"
					blurb="Production apps with room to grow. Resize machines, add replicas, expand regions."
					includes={[
						"Unlimited projects per org",
						"Resize up to 64 GB · 32 replicas / region · 500 GB volume",
						"5M requests / month included",
						"Multi-region, always-warm",
						"Custom domains, SSO, audit log, snapshots",
					]}
					cta={
						<Button asChild variant="primary" size="full">
							<Link href="/signup?plan=pro">Start Pro</Link>
						</Button>
					}
				/>
				<PlanCard
					name="Enterprise"
					price="Custom"
					blurb="Bespoke quotas, single-tenant, BYOC. For larger teams."
					includes={[
						"Custom quotas + dedicated infra",
						"Single-tenant or BYOC (AWS, GCP)",
						"Custom regions on request",
						"SLA + on-call escalation",
						"Migration assistance, security review",
					]}
					cta={
						<Button asChild variant="default" size="full">
							<a href="mailto:cloud@pylonsync.com?subject=Pylon%20Cloud%20Enterprise">
								Talk to us
							</a>
						</Button>
					}
				/>
			</div>

			<p className="mx-auto mt-10 max-w-[560px] text-center text-[13px] leading-[1.6] text-[var(--color-ink-3)]">
				Pro is $25/org/month. Bigger machines, more replicas, and larger
				volumes are billed at the underlying compute rate for what you run. Or
				self-host the open-source framework anywhere as one Pylon binary.
			</p>
		</>
	);
}

function PlanCard({
	name,
	price,
	per,
	blurb,
	includes,
	cta,
	featured,
}: {
	name: string;
	price: string;
	per?: string;
	blurb: string;
	includes: string[];
	cta: ReactNode;
	featured?: boolean;
}) {
	return (
		<div
			className={cn(
				"flex flex-col rounded-[var(--radius-lg)] border bg-[var(--color-paper-1)] p-7",
				featured
					? "border-[var(--color-cobalt)]/60"
					: "border-[var(--color-rule)]",
			)}
		>
			<div className="font-mono text-[11px] font-medium uppercase tracking-[0.12em] text-[var(--color-ink-3)]">
				{name}
			</div>
			<div className="mt-3 flex items-baseline gap-1.5 text-[36px] font-semibold leading-none tracking-[-0.03em] text-[var(--color-ink)]">
				{price}
				{per && (
					<span className="text-[13px] font-normal tracking-normal text-[var(--color-ink-3)]">
						/ {per}
					</span>
				)}
			</div>
			<p className="mt-3 text-[13.5px] leading-[1.55] text-[var(--color-ink-2)]">
				{blurb}
			</p>
			<ul className="mt-5 mb-6 flex flex-col gap-2 text-[13.5px] text-[var(--color-ink-2)]">
				{includes.map((f, i) => (
					<li key={i} className="flex items-baseline gap-2.5 leading-[1.5]">
						<span className="mt-[1px] text-[var(--color-cobalt)]">✓</span>
						<span>{f}</span>
					</li>
				))}
			</ul>
			<div className="mt-auto">{cta}</div>
		</div>
	);
}
