/**
 * Flag evaluation. Pure functions over a flag catalog + an eval
 * context. No state, no I/O — making this trivially testable +
 * cheap to call on hot paths.
 */

import type {
	BooleanFlag,
	EvalContext,
	FlagDefinition,
	FlagValue,
	MultivariateFlag,
	TargetingPredicate,
	TargetingRule,
} from "./types";

export function evaluate(
	flag: FlagDefinition,
	ctx: EvalContext,
): { value: FlagValue; matched: "default" | "targeting" | "rollout" } {
	// Targeting rules are first — they override rollout. First
	// match wins.
	if (flag.type === "boolean") {
		const ruleHit = firstMatchingRule(flag.targeting ?? [], ctx);
		if (ruleHit !== undefined) return { value: ruleHit, matched: "targeting" };
		return evalBooleanRollout(flag, ctx);
	}
	const ruleHit = firstMatchingRule(flag.targeting ?? [], ctx);
	if (ruleHit !== undefined) {
		const variant = flag.variants.find((v) => v.name === ruleHit);
		return {
			value: (variant?.payload as FlagValue) ?? ruleHit,
			matched: "targeting",
		};
	}
	return evalMultivariate(flag, ctx);
}

function evalBooleanRollout(
	flag: BooleanFlag,
	ctx: EvalContext,
): { value: boolean; matched: "default" | "rollout" } {
	if (!flag.rollout) {
		return { value: flag.default, matched: "default" };
	}
	const bucketKey = pickBucketKey(flag.rollout.key ?? "userId", ctx);
	if (!bucketKey) return { value: flag.default, matched: "default" };
	const bucket = hashBucket(bucketKey, flag.rollout.percent);
	return {
		value: bucket ? flag.default : !flag.default,
		matched: "rollout",
	};
}

function evalMultivariate(
	flag: MultivariateFlag,
	ctx: EvalContext,
): { value: string | Record<string, unknown>; matched: "default" } {
	const bucketKey = pickBucketKey("userId", ctx);
	if (!bucketKey) {
		const def = flag.variants.find((v) => v.name === flag.default);
		return {
			value: (def?.payload as Record<string, unknown>) ?? flag.default,
			matched: "default",
		};
	}
	const variant = weightedPick(flag.variants, bucketKey);
	return {
		value: (variant.payload as Record<string, unknown>) ?? variant.name,
		matched: "default",
	};
}

function firstMatchingRule<T>(
	rules: TargetingRule<T>[],
	ctx: EvalContext,
): T | undefined {
	for (const rule of rules) {
		if (rule.when.every((p) => matchPredicate(p, ctx))) return rule.value;
	}
	return undefined;
}

function matchPredicate(p: TargetingPredicate, ctx: EvalContext): boolean {
	const props = ctx.properties ?? {};
	// Specials: userId / orgId / deviceId come from the top-level
	// context fields, not the properties bag.
	let actual: unknown;
	if (p.property === "userId") actual = ctx.userId;
	else if (p.property === "orgId") actual = ctx.orgId;
	else if (p.property === "deviceId") actual = ctx.deviceId;
	else actual = props[p.property];

	switch (p.op) {
		case "eq":
			return actual === p.value;
		case "neq":
			return actual !== p.value;
		case "in":
			return Array.isArray(p.value) && p.value.includes(actual as never);
		case "not_in":
			return Array.isArray(p.value) && !p.value.includes(actual as never);
		case "gt":
			return typeof actual === "number" && actual > (p.value as number);
		case "gte":
			return typeof actual === "number" && actual >= (p.value as number);
		case "lt":
			return typeof actual === "number" && actual < (p.value as number);
		case "lte":
			return typeof actual === "number" && actual <= (p.value as number);
		case "contains":
			return typeof actual === "string" && actual.includes(p.value as string);
		case "starts_with":
			return typeof actual === "string" && actual.startsWith(p.value as string);
		case "ends_with":
			return typeof actual === "string" && actual.endsWith(p.value as string);
		case "regex":
			try {
				return (
					typeof actual === "string" &&
					new RegExp(p.value as string).test(actual)
				);
			} catch {
				return false;
			}
	}
	return false;
}

function pickBucketKey(
	keyName: string,
	ctx: EvalContext,
): string | undefined {
	if (keyName === "userId") return ctx.userId;
	if (keyName === "orgId") return ctx.orgId;
	if (keyName === "deviceId") return ctx.deviceId;
	const v = ctx.properties?.[keyName];
	return typeof v === "string" ? v : undefined;
}

/**
 * Deterministic bucketing. The hash is FNV-1a (cheap, well-
 * distributed) over the bucketKey; map to 0..99 and compare against
 * the rollout percent.
 *
 * Why not SHA-256: bucket calls are on the hot path (every
 * isEnabled() check), and FNV-1a gives near-uniform distribution
 * for the input domain (32-char IDs) at a fraction of the cost.
 * Same primitive PostHog uses for local-eval bucketing.
 */
export function hashBucket(key: string, percent: number): boolean {
	let h = 0x811c9dc5;
	for (let i = 0; i < key.length; i++) {
		h ^= key.charCodeAt(i);
		h = Math.imul(h, 0x01000193);
	}
	const bucket = ((h >>> 0) % 10000) / 100; // 0..99.99
	return bucket < percent;
}

function weightedPick<T extends { name: string; weight: number }>(
	variants: T[],
	key: string,
): T {
	const totalWeight = variants.reduce((s, v) => s + v.weight, 0);
	let h = 0x811c9dc5;
	for (let i = 0; i < key.length; i++) {
		h ^= key.charCodeAt(i);
		h = Math.imul(h, 0x01000193);
	}
	const point = ((h >>> 0) % totalWeight) + 1;
	let acc = 0;
	for (const v of variants) {
		acc += v.weight;
		if (point <= acc) return v;
	}
	return variants[variants.length - 1];
}
