/**
 * `@pylonsync/feature-flags` — local-eval feature flags for Pylon
 * apps. Boolean + multivariate flags, percentage rollouts, targeting
 * rules, JSON payloads per variant, kill switches.
 *
 * Local-eval means there's no network call per check — flag
 * definitions are loaded into the process once at startup, and
 * `isEnabled(flag, ctx)` is a deterministic in-memory computation.
 * That's the same model PostHog/LaunchDarkly local-eval mode uses
 * and is what makes flag checks viable on hot request paths
 * (e.g. inside auth or middleware).
 *
 * Flag definitions live in the app's `FeatureFlag` entity (declared
 * via the manifest fragment); apps mutate them via the dashboard
 * actions exposed in `handlers/`. The plugin caches the flag list
 * in-process and invalidates on `setFlag` mutations.
 */

export type FlagValue = boolean | string | Record<string, unknown>;

export interface BooleanFlag {
	type: "boolean";
	default: boolean;
	/**
	 * Rollout config. If absent, the default value is returned for
	 * everyone. If `percent` is set, `default` is the value for
	 * users in the rollout; non-rollout users get `!default`.
	 */
	rollout?: RolloutConfig;
	/** Optional targeting rules — first match wins. */
	targeting?: TargetingRule<boolean>[];
}

export interface MultivariateFlag {
	type: "multivariate";
	/** Variant catalog. Weights must sum to 100. */
	variants: Array<{ name: string; weight: number; payload?: unknown }>;
	/** Default variant name when no targeting rule matches. */
	default: string;
	/** Optional targeting rules — first match wins. */
	targeting?: TargetingRule<string>[];
}

export type FlagDefinition = BooleanFlag | MultivariateFlag;

export interface RolloutConfig {
	/** 0..100 — percentage of distinct-ids that get the rollout. */
	percent: number;
	/**
	 * Hashing key used to bucket users. Default `userId`. Set to
	 * `orgId` for per-tenant rollouts (every member of the tenant
	 * sees the same value), or a custom key for cohort experiments.
	 */
	key?: "userId" | "orgId" | "deviceId" | string;
}

export interface TargetingRule<TValue> {
	value: TValue;
	when: TargetingPredicate[];
}

/**
 * Composable predicate. Multiple predicates AND together. Each
 * compares a `property` (looked up on the eval context) to a
 * literal `value` via an operator.
 */
export interface TargetingPredicate {
	property: string;
	op:
		| "eq"
		| "neq"
		| "in"
		| "not_in"
		| "gt"
		| "gte"
		| "lt"
		| "lte"
		| "contains"
		| "starts_with"
		| "ends_with"
		| "regex";
	value: string | number | boolean | Array<string | number>;
}

export interface EvalContext {
	/** Stable user identifier — drives default bucketing. */
	userId?: string;
	orgId?: string;
	deviceId?: string;
	/**
	 * Free-form properties consulted by targeting rules. Apps
	 * stuff whatever signal they want here (`country`, `plan`,
	 * `betaTester`, `signupDaysAgo`, etc.).
	 */
	properties?: Record<string, unknown>;
	/**
	 * Server-side `now` injectable for tests. Defaults to
	 * `Date.now()`.
	 */
	now?: number;
}

export interface FlagPluginConfig {
	/**
	 * Inline flag catalog. Apps that want their flags in code (e.g.
	 * to keep them in version control) declare them here. Apps that
	 * want runtime-editable flags omit this and use the
	 * `FeatureFlag` entity instead.
	 */
	flags?: Record<string, FlagDefinition>;
	/**
	 * Database-backed flags. When true, the manifest fragment adds
	 * a `FeatureFlag` entity + a `setFlag`/`deleteFlag` admin
	 * mutation pair. The runtime merges inline + database flags
	 * with database winning.
	 */
	editable?: boolean;
	/** Entity name override. */
	entityName?: string;
}
