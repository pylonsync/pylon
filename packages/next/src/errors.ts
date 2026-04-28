/**
 * Shared error type for Pylon API calls. Lives outside `client.ts` (which
 * is `"use client"`) so server code can throw + catch the same class
 * without dragging the whole client bundle in.
 *
 * Carries the wire `code` (e.g. `OAUTH_INVALID_STATE`) so UI can branch
 * on specific failures instead of string-matching the message.
 */
export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string,
	) {
		super(message);
		this.name = "ApiError";
	}
}
