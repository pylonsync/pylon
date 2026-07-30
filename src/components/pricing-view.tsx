"use client";

import { Link } from "@pylonsync/react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { PricingPlans } from "./pricing-plans";

const FAQS: { q: string; a: string }[] = [
	{
		q: "Is the framework free?",
		a: "Yes. Pylon is open source under MIT/Apache and free to self-host. It runs as one binary, so a small VPS is enough. Smallware is the optional managed service.",
	},
	{
		q: "What counts as a request?",
		a: "Any HTTP request or sync message your app serves. A WebSocket connection that streams diffs counts far less than equivalent polling, so realtime apps go a long way on the included quota.",
	},
	{
		q: "What happens if I exceed my plan?",
		a: "Bigger machines, extra replicas, and larger volumes are billed at the underlying compute rate for what you run, on top of the plan fee. Idle machines can autostop and resume on the next request. There are no per-seat fees.",
	},
	{
		q: "Can I move between self-hosted and Cloud?",
		a: "Yes. Cloud runs the same binary you run locally. Deploy to your own VPS, to Cloud, or to both without changing the app.",
	},
	{
		q: "Do you offer SSO and SAML?",
		a: "On every paid plan. Configure OIDC or SAML SSO at the org level from the dashboard. Magic-link and 25+ OAuth providers are available throughout.",
	},
	{
		q: "What's included in Enterprise?",
		a: "Custom quotas, dedicated or single-tenant infrastructure, bring-your-own-cloud (AWS, GCP), custom regions, an SLA with on-call escalation, plus migration help and a security review.",
	},
];

export function PricingView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[900px] px-5 pb-12 pt-20 text-center sm:px-8 sm:pb-14 sm:pt-28">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
						Pricing
					</div>
					<h1 className="mx-auto mt-4 max-w-[18ch] text-[clamp(34px,5vw,60px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						One price. Scale when you do.
					</h1>
					<p className="mx-auto mt-6 max-w-[52ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						The framework is open source and free to self-host. Smallware adds
						managed hosting, scaling, and a dashboard — $25 per org for one
						person, $99 for a team. No per-seat fees.
					</p>
				</div>
			</header>

			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1100px] px-5 py-16 sm:px-8 sm:py-20">
					<PricingPlans signedIn={signedIn} />
				</div>
			</section>

			{/* FAQ */}
			<section>
				<div className="mx-auto max-w-[820px] px-5 py-16 sm:px-8 sm:py-20">
					<h2 className="text-center text-[clamp(26px,3.5vw,40px)] font-semibold tracking-[-0.03em] text-[var(--color-ink)]">
						Questions
					</h2>
					<div className="mt-10 flex flex-col divide-y divide-[var(--color-rule)] border-y border-[var(--color-rule)]">
						{FAQS.map((f) => (
							<div key={f.q} className="grid gap-2 py-6 sm:grid-cols-[1fr_1.4fr] sm:gap-8">
								<h3 className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
									{f.q}
								</h3>
								<p className="text-[14px] leading-[1.6] text-[var(--color-ink-2)]">
									{f.a}
								</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* CTA */}
			<section className="border-t border-[var(--color-rule)]">
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[16ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Ship your backend today.
					</h2>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={signedIn ? "/dashboard" : "/signup"}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<a href="https://docs.pylonsync.com">Read the docs</a>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}
