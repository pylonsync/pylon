"use client";

import { type ReactNode, useEffect, useState } from "react";
import { useAuth } from "../hooks/useAuth";
import { acceptInvite, ApiError } from "../lib/api";
import { cn } from "../lib/cn";

export interface AcceptInviteProps {
	/**
	 * The invite token. Pulled from the email link's `?token=` param
	 * by the consuming page. Falls back to `window.location.search`
	 * when omitted.
	 */
	token?: string;
	/** Where to send the user after a successful accept. */
	afterAcceptUrl?: string;
	/** Where to send a signed-out user so they can sign in / sign up first. */
	signInUrl?: string;
	/** Optional callback once accept lands. Receives `{org_id, role}`. */
	onAccepted?: (result: { org_id: string; role: string }) => void;
	/** Override the headline copy. */
	title?: ReactNode;
	className?: string;
}

type State =
	| { kind: "idle" }
	| { kind: "accepting" }
	| { kind: "accepted"; orgId: string; role: string }
	| { kind: "error"; message: string };

/**
 * Drop-in landing surface for `/accept-invite?token=…`. Reads the token
 * from props or the URL, calls `POST /api/auth/invites/:token/accept`,
 * and lights up org membership when the request succeeds.
 *
 * Signed-out users see a "sign in to accept" prompt that bounces through
 * `signInUrl` with the current href as `?next=` so they end up back on
 * this page with the token preserved.
 */
export function AcceptInvite({
	token: tokenProp,
	afterAcceptUrl,
	signInUrl = "/sign-in",
	onAccepted,
	title = "Accept invite",
	className,
}: AcceptInviteProps) {
	const { isSignedIn, refresh } = useAuth();
	const [state, setState] = useState<State>({ kind: "idle" });

	const token =
		tokenProp ??
		(typeof window !== "undefined"
			? new URLSearchParams(window.location.search).get("token")
			: null) ??
		undefined;

	useEffect(() => {
		if (!token || !isSignedIn) return;
		// Auto-accept once we have both a token and a session. Saves the
		// "click accept" step that doesn't add any safety — the user is
		// already on the URL they were emailed.
		setState({ kind: "accepting" });
		(async () => {
			try {
				const result = await acceptInvite(token);
				setState({ kind: "accepted", orgId: result.org_id, role: result.role });
				onAccepted?.(result);
				// Refresh the cached session so the new tenantId / role is
				// visible in useAuth() immediately. Without this the user
				// would need to reload to see the org switcher pick up the
				// membership.
				void refresh();
				if (afterAcceptUrl && typeof window !== "undefined") {
					window.location.assign(afterAcceptUrl);
				}
			} catch (err) {
				setState({ kind: "error", message: messageFromError(err) });
			}
		})();
	}, [token, isSignedIn, afterAcceptUrl, onAccepted, refresh]);

	return (
		<div
			className={cn(
				"mx-auto w-full max-w-sm space-y-4 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-7 text-center shadow-sm",
				className,
			)}
		>
			<h2 className="text-lg font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
				{title}
			</h2>
			{!token ? (
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					This page needs an invite token. Check the email link you
					were sent.
				</p>
			) : !isSignedIn ? (
				<SignInPrompt signInUrl={signInUrl} />
			) : state.kind === "idle" || state.kind === "accepting" ? (
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					Joining…
				</p>
			) : state.kind === "accepted" ? (
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					You're in. Welcome to the team.
				</p>
			) : (
				<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
					{state.message}
				</p>
			)}
		</div>
	);
}

function SignInPrompt({ signInUrl }: { signInUrl: string }) {
	const next =
		typeof window !== "undefined" && window.location?.href
			? `?next=${encodeURIComponent(window.location.href)}`
			: "";
	return (
		<>
			<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
				Sign in to accept your invite. We'll bring you right back.
			</p>
			<a
				href={`${signInUrl}${next}`}
				className="inline-flex items-center justify-center rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-4 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)]"
			>
				Sign in
			</a>
		</>
	);
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "INVITE_NOT_FOUND":
				return "This invite link is no longer valid.";
			case "INVITE_EXPIRED":
				return "This invite has expired. Ask for a fresh one.";
			case "ALREADY_ACCEPTED":
				return "You've already accepted this invite.";
			case "WRONG_EMAIL":
				return "This invite was for a different email. Sign in with the account that received it.";
			case "ALREADY_MEMBER":
				return "You're already a member of this org.";
			case "NO_EMAIL":
				return "Your account has no email on file — add one and try again.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong accepting the invite.";
}
