/**
 * Token-bucket rate limiter for API keys.
 *
 * Model: each key has a bucket with `max` tokens. Each request
 * consumes one token. Tokens refill linearly at rate
 * `max / windowSecs` (so `100/60` = ~1.67 tokens per second).
 * When the bucket hits 0, requests are rejected until tokens
 * accumulate again.
 *
 * Token-bucket is friendlier than fixed-window because a burst
 * just spends accumulated tokens instead of hitting a hard cliff
 * at the window boundary. Customer apps prefer this for the
 * "spike of usage at the start of the minute" pattern.
 *
 * Persistence: caller passes in the bucket state from storage,
 * gets a decision back + the new bucket state to persist. Stateless
 * by design so the limiter scales across multiple processes —
 * synchronization is the caller's job (use a DB row update with
 * compare-and-swap, or Redis INCR with TTL).
 */

export interface RateLimitInput {
	/** Bucket capacity = max requests in a window. */
	max: number;
	/** Window length in seconds (defines refill rate = max/window). */
	windowSecs: number;
	/** Current tokens. */
	tokens: number;
	/** Last time the bucket was touched (ms since epoch). */
	updatedAt: number;
	/** Now (ms since epoch). Pluggable for tests. */
	now: number;
}

export interface RateLimitDecision {
	allowed: boolean;
	/** Tokens remaining after consumption (or pre-consumption on deny). */
	tokens: number;
	/** New `updatedAt` to persist. */
	updatedAt: number;
}

/**
 * Refill the bucket based on elapsed time, then try to consume one
 * token. Returns the new state.
 */
export function checkAndConsume(input: RateLimitInput): RateLimitDecision {
	const elapsedMs = Math.max(0, input.now - input.updatedAt);
	const refillRate = input.max / (input.windowSecs * 1000); // tokens / ms
	const refilled = Math.min(input.max, input.tokens + elapsedMs * refillRate);

	if (refilled < 1) {
		return {
			allowed: false,
			tokens: refilled,
			updatedAt: input.now,
		};
	}
	return {
		allowed: true,
		tokens: refilled - 1,
		updatedAt: input.now,
	};
}

export interface RateLimitState {
	tokens: number;
	updatedAt: string;
	max: number;
	windowSecs: number;
}
