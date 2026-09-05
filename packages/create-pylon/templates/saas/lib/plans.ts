// The plan catalog. One place for prices, the free-tier cap, the trial
// length, and the feature bullets, read by the marketing pricing page, the
// Billing tab, the Stripe plugin config (lib/billing.ts), and the server
// function that enforces the cap (functions/createProject.ts).

/** Active projects a free workspace can have. `createProject` enforces it. */
export const FREE_PROJECT_LIMIT = 3;

/** Days of free Pro on the first subscription. Stripe collects a card up front. */
export const TRIAL_DAYS = 14;

export type PlanId = "free" | "pro";

export interface PlanDefinition {
  id: PlanId;
  name: string;
  tagline: string;
  /** USD per month, billed monthly. */
  monthly: number;
  /** USD per month when billed annually (null = no annual price). */
  annualPerMonth: number | null;
  features: string[];
  cta: string;
}

export const PLANS: PlanDefinition[] = [
  {
    id: "free",
    name: "Free",
    tagline: "For getting started.",
    monthly: 0,
    annualPerMonth: null,
    features: [
      `Up to ${FREE_PROJECT_LIMIT} active projects`,
      "Unlimited members",
      "Real-time collaboration",
      "Community support",
    ],
    cta: "Get started",
  },
  {
    id: "pro",
    name: "Pro",
    tagline: "For teams shipping every week.",
    monthly: 29,
    annualPerMonth: 24,
    features: [
      "Unlimited projects",
      "Unlimited members",
      "Priority support",
      "Everything we ship next",
    ],
    cta: `Start ${TRIAL_DAYS}-day free trial`,
  },
];

export function planById(id: string): PlanDefinition | undefined {
  return PLANS.find((p) => p.id === id);
}

/** "$29" / "$0". */
export function formatPrice(usd: number): string {
  return `$${usd}`;
}

/** Whole-year price for the annual option, e.g. 24 → 288. */
export function annualTotal(plan: PlanDefinition): number | null {
  return plan.annualPerMonth == null ? null : plan.annualPerMonth * 12;
}

/** Percent saved by paying annually, rounded. */
export function annualSavingsPercent(plan: PlanDefinition): number {
  if (plan.annualPerMonth == null || plan.monthly === 0) return 0;
  return Math.round((1 - plan.annualPerMonth / plan.monthly) * 100);
}

/**
 * Can a workspace on `plan` with `activeProjects` create one more?
 * Pure so the cap is unit-tested; `createProject` calls it.
 */
export function canCreateProject(plan: PlanId, activeProjects: number): boolean {
  if (plan === "pro") return true;
  return activeProjects < FREE_PROJECT_LIMIT;
}

/** A subscription counts as Pro while Stripe reports it usable. */
export const ACTIVE_STATUSES = ["active", "trialing", "past_due"];

export function planFromSubscription(
  sub: { plan?: string; status?: string } | null | undefined,
): PlanId {
  if (!sub) return "free";
  return ACTIVE_STATUSES.includes(sub.status ?? "") && sub.plan === "pro" ? "pro" : "free";
}
