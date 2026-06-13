"use client";
import React from "react";

import { type ReactNode, useState } from "react";
import { useAuth } from "../hooks/useAuth";
import { ApiError, connectionAuthUrl } from "../lib/api";
import { cn } from "../lib/cn";

export interface ConnectAccountProps {
	/**
	 * Name of the connection in the manifest (matches the `name` you
	 * passed to `defineConnection({...})`). NOT the OAuth provider id.
	 */
	name: string;
	/**
	 * URL the OAuth provider sends the user back to after consent.
	 * Defaults to the current page so the user lands on the same screen
	 * with the new connection already in place.
	 */
	postRedirect?: string;
	/** Optional label override. Default: "Connect <name>". */
	label?: ReactNode;
	/**
	 * If true, render an "outline" style button instead of the solid
	 * primary. Useful when the connect-button sits next to a primary
	 * CTA.
	 */
	variant?: "primary" | "outline";
	className?: string;
}

/**
 * Drop-in button that starts an OAuth flow for a connection defined via
 * `defineConnection(...)` in the manifest. Hits
 * `POST /api/connections/<name>/auth-url` and navigates the browser to
 * the returned URL; the provider redirects back to
 * `/api/connections/<name>/callback`, which stores the encrypted token
 * row and bounces to `postRedirect` (default: current page).
 *
 * Renders nothing for signed-out users — connections need an
 * authenticated identity to bind the access token to.
 */
export function ConnectAccount({
	name,
	postRedirect,
	label,
	variant = "primary",
	className,
}: ConnectAccountProps) {
	const { isSignedIn } = useAuth();
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	if (!isSignedIn) return null;

	async function onClick() {
		setError(null);
		setPending(true);
		try {
			const redirect =
				postRedirect ??
				(typeof window !== "undefined" ? window.location.href : undefined);
			const { url } = await connectionAuthUrl(name, redirect);
			if (typeof window !== "undefined") {
				window.location.assign(url);
			}
		} catch (err) {
			setError(messageFromError(err));
			setPending(false);
		}
	}

	return (
		<span className={cn("inline-flex flex-col gap-1.5", className)}>
			<button
				type="button"
				onClick={onClick}
				disabled={pending}
				className={cn(
					"inline-flex items-center justify-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition-colors disabled:opacity-60",
					variant === "primary"
						? "bg-[var(--pylon-ink,#0a0a0a)] text-[var(--pylon-paper,#ffffff)] hover:opacity-90"
						: "border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] text-[var(--pylon-ink,#0a0a0a)] hover:bg-[var(--pylon-paper-2,#f4f4f5)]",
				)}
			>
				{pending ? "…" : (label ?? `Connect ${capitalize(name)}`)}
			</button>
			{error ? (
				<span className="text-[11px] text-[var(--pylon-error-ink,#b91c1c)]">
					{error}
				</span>
			) : null}
		</span>
	);
}

function capitalize(s: string): string {
	if (!s) return "";
	return s.charAt(0).toUpperCase() + s.slice(1);
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "CONNECTION_UNKNOWN":
				return "This connection isn't defined in the app manifest.";
			case "PROVIDER_NOT_CONFIGURED":
				return "Connection provider isn't configured on the server.";
			case "ENCRYPTION_REQUIRED":
				return "Connection storage needs PYLON_ENCRYPTION_KEY set.";
			case "CONNECTIONS_NOT_CONFIGURED":
				return "No connections are defined in this app yet.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Couldn't start the connection flow.";
}
