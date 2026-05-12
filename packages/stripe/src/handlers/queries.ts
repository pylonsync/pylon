import { mutation, query, v } from "@pylonsync/functions";

import type { StripeConfig } from "../types";
import { subscriptionEntity } from "./internal";

/**
 * Plugin-internal queries + mutations. Names are namespaced with a
 * `_pylonStripe` prefix to avoid colliding with app functions. The
 * underscore + `internal: true` makes them ineligible for direct
 * HTTP invocation — they only fire via `ctx.runQuery` /
 * `ctx.runMutation` from the action handler factories.
 *
 * Each one is parameterized over the entity names from `cfg.entities`
 * so apps can rename `StripeSubscription` → `Subscription` or
 * `Org` → `Workspace` without forking the plugin. Entity names that
 * the framework needs as relation targets (`v.id("Org")`) can't be
 * dynamic, so the lookup helpers take the entity name as an arg and
 * use the generic `ctx.db.query(entity, filter)` API.
 */

export function internalHandlers(cfg: StripeConfig): Record<string, unknown> {
	const subEnt = subscriptionEntity(cfg);

	return {
		_pylonStripeListSubsForReference: query({
			args: { referenceId: v.string() },
			internal: true,
			async handler(ctx, args: { referenceId: string }) {
				return ctx.db.query(subEnt, { referenceId: args.referenceId });
			},
		}),

		// First active subscription for a reference id (used by cancel
		// + restore). "Active" here means status ∈
		// (active, trialing, past_due) — those are the states a
		// customer can still mutate from. Canceled / unpaid require
		// a new checkout.
		_pylonStripeFindActiveSubForReference: query({
			args: { referenceId: v.string() },
			internal: true,
			async handler(ctx, args: { referenceId: string }) {
				const rows = (await ctx.db.query(subEnt, {
					referenceId: args.referenceId,
				})) as Array<{
					id: string;
					stripeSubscriptionId: string;
					status: string;
					cancelAtPeriodEnd?: boolean;
				}>;
				return (
					rows.find((r) =>
						["active", "trialing", "past_due"].includes(r.status),
					) ?? null
				);
			},
		}),

		// Read the customer-holder row. Used by resolveCustomerForReference
		// to look up the existing stripeCustomerId + carry email/name
		// through to customer creation.
		_pylonStripeGetCustomerHolder: query({
			args: { entity: v.string(), id: v.string() },
			internal: true,
			async handler(ctx, args: { entity: string; id: string }) {
				return ctx.db.get(args.entity, args.id);
			},
		}),

		// Reverse lookup: customer-id → reference row. Used by the
		// webhook handler to map Stripe events back to our reference.
		_pylonStripeFindByCustomerId: query({
			args: {
				entity: v.string(),
				stripeCustomerId: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: { entity: string; stripeCustomerId: string },
			) {
				const rows = (await ctx.db.query(args.entity, {
					stripeCustomerId: args.stripeCustomerId,
				})) as Array<{ id: string }>;
				return rows[0] ?? null;
			},
		}),

		// Org membership check for the default `authorizeReference`.
		// Apps with non-default OrgMember entity names pass their own
		// authorizer instead.
		_pylonStripeOrgMembership: query({
			args: { orgId: v.string(), userId: v.string() },
			internal: true,
			async handler(ctx, args: { orgId: string; userId: string }) {
				return ctx.db.query("OrgMember", {
					orgId: args.orgId,
					userId: args.userId,
				});
			},
		}),

		// Persist a freshly-minted Stripe customer id on the holder row.
		_pylonStripeSetCustomerId: mutation({
			args: {
				entity: v.string(),
				id: v.string(),
				stripeCustomerId: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: { entity: string; id: string; stripeCustomerId: string },
			) {
				await ctx.db.update(args.entity, args.id, {
					stripeCustomerId: args.stripeCustomerId,
				});
			},
		}),

		// Upsert the subscription row from a webhook event. Looks up
		// by stripeSubscriptionId (unique) and updates in place when
		// it exists; otherwise inserts. Idempotent — Stripe retries
		// webhooks and we want the same end state on repeat delivery.
		_pylonStripeUpsertSubscription: mutation({
			args: {
				referenceId: v.string(),
				stripeCustomerId: v.string(),
				stripeSubscriptionId: v.string(),
				plan: v.string(),
				status: v.string(),
				seats: v.optional(v.number()),
				currentPeriodEnd: v.optional(v.string()),
				cancelAtPeriodEnd: v.optional(v.boolean()),
				canceledAt: v.optional(v.string()),
				trialEnd: v.optional(v.string()),
				limits: v.optional(v.string()),
				createdAt: v.string(),
				updatedAt: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: {
					referenceId: string;
					stripeCustomerId: string;
					stripeSubscriptionId: string;
					plan: string;
					status: string;
					seats?: number;
					currentPeriodEnd?: string | null;
					cancelAtPeriodEnd?: boolean;
					canceledAt?: string | null;
					trialEnd?: string | null;
					limits?: string | null;
					createdAt: string;
					updatedAt: string;
				},
			) {
				const existing = (await ctx.db.query(subEnt, {
					stripeSubscriptionId: args.stripeSubscriptionId,
				})) as Array<{ id: string }>;
				if (existing[0]) {
					await ctx.db.update(subEnt, existing[0].id, {
						plan: args.plan,
						status: args.status,
						seats: args.seats ?? 1,
						currentPeriodEnd: args.currentPeriodEnd ?? null,
						cancelAtPeriodEnd: args.cancelAtPeriodEnd ?? false,
						canceledAt: args.canceledAt ?? null,
						trialEnd: args.trialEnd ?? null,
						limits: args.limits ?? null,
						updatedAt: args.updatedAt,
					});
				} else {
					await ctx.db.insert(subEnt, {
						referenceId: args.referenceId,
						stripeCustomerId: args.stripeCustomerId,
						stripeSubscriptionId: args.stripeSubscriptionId,
						plan: args.plan,
						status: args.status,
						seats: args.seats ?? 1,
						currentPeriodEnd: args.currentPeriodEnd ?? null,
						cancelAtPeriodEnd: args.cancelAtPeriodEnd ?? false,
						canceledAt: args.canceledAt ?? null,
						trialEnd: args.trialEnd ?? null,
						limits: args.limits ?? null,
						createdAt: args.createdAt,
						updatedAt: args.updatedAt,
					});
				}
			},
		}),
	};
}
