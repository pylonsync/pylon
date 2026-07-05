// SessionResolver — the engine's identity state machine.
//
// Before extraction, four pieces of state (`_resolvedSession`,
// `lastSeenToken`, `lastSeenTenant`, `sessionSignature`) and three
// branching rules (null→X first-resolution skip, X→Y reset, token-flip
// reset) were sprinkled across four methods in index.ts. Each call site
// re-implemented the comparison. This module collects the rules into
// one place so the engine just feeds observations in and reads back
// verdicts.
//
// No engine dependencies — pure state. Unit-tested in isolation.

import type { ResolvedSession } from "./types";

/** Verdict returned from `observeSession`. The engine acts on these
 *  three booleans; the resolver makes no decisions about side effects
 *  (replica reset, store notify, pull) — those belong upstream. */
export interface SessionTransition {
  /** Resolved session differs in any field (userId / tenantId / isAdmin /
   *  roles). When true, the engine should notify subscribers. */
  identityChanged: boolean;
  /** The tenant moved AND it isn't the null→X first-resolution case —
   *  cached rows belong to a different tenant and the replica should
   *  be wiped before the next pull. */
  replicaInvalidated: boolean;
  /** First time tenant flipped from null to a real value (the engine
   *  started before /api/auth/select-org landed). The cached rows in
   *  this case ARE for the new tenant; reset would tombstone valid
   *  state. Engine should pull but NOT reset. */
  isFirstResolution: boolean;
  /** Tenant value moved at all (covers both null→X and X→Y). When
   *  true, the engine should pull regardless of reset semantics —
   *  cached rows under the old tenant won't include rows added under
   *  the new one. */
  tenantChanged: boolean;
}

export interface TokenTransition {
  /** Token flipped since the last observation. Engine should reset
   *  the replica because the visible set changed under a new identity. */
  tokenChanged: boolean;
}

const EMPTY_SESSION: ResolvedSession = {
  userId: null,
  tenantId: null,
  isAdmin: false,
  roles: [],
};

export class SessionResolver {
  private _resolved: ResolvedSession = EMPTY_SESSION;
  private _observed = false;
  /** `undefined` until the first observation — distinguishes "we've
   *  never seen a token" from "the token is null." Same for tenant. */
  private lastSeenToken: string | null | undefined = undefined;
  private lastSeenTenant: string | null | undefined = undefined;

  /** Current resolved session — what `useSession` consumers should see. */
  resolved(): ResolvedSession {
    return this._resolved;
  }

  /** Stable string signature of the current session. Captured before
   *  long-running ops (reconcile, in particular) so the op can detect
   *  if anything changed during its in-flight period and bail. */
  signature(): string {
    return sessionSignature(this._resolved);
  }

  /** Compute the verdict for a freshly resolved session WITHOUT
   *  mutating internal state. The engine inspects the verdict to
   *  decide whether to reset the replica + pull, then calls
   *  `commitObservation(next)` once it's safe for `useSession`
   *  subscribers to see the new tenant.
   *
   *  Splitting compute from commit prevents the "useSession reports
   *  new tenant + useQuery shows old tenant's rows" inconsistency
   *  window that existed when the resolved session was mutated
   *  before `resetReplica()` finished. */
  inspectSession(next: ResolvedSession): SessionTransition {
    const prev = this._resolved;
    const tenantNow = next.tenantId;

    const identityChanged = sessionSignature(prev) !== sessionSignature(next);
    const tenantChanged =
      this.lastSeenTenant !== undefined && this.lastSeenTenant !== tenantNow;
    const isFirstResolution =
      this.lastSeenTenant === null && tenantNow !== null;
    const replicaInvalidated = tenantChanged && !isFirstResolution;

    return {
      identityChanged,
      replicaInvalidated,
      isFirstResolution,
      tenantChanged,
    };
  }

  /** Commit a previously-inspected session as the current truth.
   *  Engine calls this AFTER acting on the verdict (replica reset,
   *  pull) so subscribers never observe a half-applied transition. */
  commitObservation(next: ResolvedSession): void {
    this.lastSeenTenant = next.tenantId;
    this._resolved = next;
  }

  /** Convenience for tests / migration: inspect + commit in one call.
   *  Production callers should use `inspectSession` + `commitObservation`
   *  to control the timing of state mutation. */
  observeSession(next: ResolvedSession): SessionTransition {
    const verdict = this.inspectSession(next);
    this.commitObservation(next);
    this._observed = true;
    return verdict;
  }

  /** Has /api/auth/me actually resolved THIS run? False on an offline
   *  start — callers that would destroy state on an identity mismatch
   *  (replica wipes) must not act on the EMPTY placeholder session. */
  hasObserved(): boolean {
    return this._observed;
  }

  /** Feed in the current bearer token. Returns whether it flipped
   *  since the previous observation. The engine uses this in pull()
   *  to decide whether to reset before reading the cursor. */
  observeToken(token: string | null): TokenTransition {
    const tokenChanged =
      this.lastSeenToken !== undefined && this.lastSeenToken !== token;
    this.lastSeenToken = token;
    return { tokenChanged };
  }
}

/** Stable signature of a resolved session. Used by reconcile to detect
 *  mid-fetch session flips. Roles array is sorted+joined so insertion
 *  order doesn't trip the equality check. */
export function sessionSignature(s: ResolvedSession): string {
  const roles = (s.roles ?? []).slice().sort().join(",");
  return `${s.userId ?? ""}|${s.tenantId ?? ""}|${s.isAdmin ? "1" : "0"}|${roles}`;
}
