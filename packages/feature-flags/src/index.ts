/**
 * Public API.
 *
 * Two surfaces:
 *   - Inline catalog: declare flags in TS, evaluate via `isEnabled` /
 *     `getVariant` / `getPayload` over a process-local `flags` object.
 *   - Editable catalog: stored in a `FeatureFlag` entity, mutated via
 *     admin actions, cached in-process. Useful for kill switches that
 *     ops needs to flip without a redeploy.
 *
 * The plugin doesn't ship an A/B testing analyzer — that's a different
 * concern (compute statistical significance from event logs). Pair
 * with @pylonsync/audit-log to record `flag.assigned` events for
 * downstream analysis.
 */

export { evaluate, hashBucket } from "./eval";
export type {
	BooleanFlag,
	MultivariateFlag,
	FlagDefinition,
	FlagValue,
	RolloutConfig,
	TargetingRule,
	TargetingPredicate,
	EvalContext,
	FlagPluginConfig,
} from "./types";

import { evaluate } from "./eval";
import type { EvalContext, FlagDefinition } from "./types";

/**
 * Boolean check. Returns the resolved boolean for the flag in the
 * given eval context. Unknown flag names return `false` (closed by
 * default — apps that want different semantics for unknown keys
 * check via `getRawValue`).
 */
export function isEnabled(
	flags: Record<string, FlagDefinition>,
	name: string,
	ctx: EvalContext,
): boolean {
	const def = flags[name];
	if (!def || def.type !== "boolean") return false;
	const r = evaluate(def, ctx);
	return Boolean(r.value);
}

/**
 * Multivariate check. Returns the variant name (or default if the
 * flag is missing / typed wrong).
 */
export function getVariant(
	flags: Record<string, FlagDefinition>,
	name: string,
	ctx: EvalContext,
): string {
	const def = flags[name];
	if (!def || def.type !== "multivariate") return "";
	const r = evaluate(def, ctx);
	return typeof r.value === "string" ? r.value : "";
}

/**
 * Variant payload (JSON config). Useful for remote-config style flags
 * where each variant carries different settings (rate limits, model
 * choice, copy strings).
 */
export function getPayload(
	flags: Record<string, FlagDefinition>,
	name: string,
	ctx: EvalContext,
): unknown {
	const def = flags[name];
	if (!def) return undefined;
	const r = evaluate(def, ctx);
	return r.value;
}

/**
 * Bulk evaluation — useful for SSR bootstrapping where the client
 * receives every flag's resolved value with the initial page load.
 * Returns `{ [flagName]: value }`.
 */
export function evaluateAll(
	flags: Record<string, FlagDefinition>,
	ctx: EvalContext,
): Record<string, unknown> {
	const out: Record<string, unknown> = {};
	for (const [name, def] of Object.entries(flags)) {
		out[name] = evaluate(def, ctx).value;
	}
	return out;
}
