"use client";
import React from "react";

import { type FormEvent, type ReactNode, useEffect, useState } from "react";
import {
	ApiError,
	completePasswordReset,
	requestPasswordReset,
} from "../lib/api";
import { cn } from "../lib/cn";

// ---------------------------------------------------------------------------
// <ForgotPassword />
// ---------------------------------------------------------------------------

export interface ForgotPasswordProps {
	/** Return route shown as "back to sign-in" link. */
	signInUrl?: string;
	/** Override headline. */
	title?: ReactNode;
	className?: string;
}

/**
 * "Forgot password" entry point. POSTs `/api/auth/password/reset/request`
 * and always shows a generic success screen — the server returns 200
 * regardless of whether the email is registered (account-enumeration
 * defense), so we mirror that here.
 */
export function ForgotPassword({
	signInUrl = "/sign-in",
	title = "Reset your password",
	className,
}: ForgotPasswordProps) {
	const [email, setEmail] = useState("");
	const [sent, setSent] = useState(false);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			await requestPasswordReset(email);
			setSent(true);
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<Card className={className}>
			<Heading title={title} />
			{sent ? (
				<>
					<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
						If an account exists for{" "}
						<span className="font-medium text-[var(--pylon-ink,#0a0a0a)]">
							{email}
						</span>
						, we sent a reset link.
					</p>
					<a
						href={signInUrl}
						className="block text-center text-xs text-[var(--pylon-ink-2,#52525b)] hover:underline"
					>
						Back to sign in
					</a>
				</>
			) : (
				<form onSubmit={onSubmit} className="space-y-3">
					<Field
						label="Email"
						type="email"
						value={email}
						onChange={setEmail}
						required
						autoComplete="email"
						placeholder="you@example.com"
					/>
					<button
						type="submit"
						disabled={pending}
						className="w-full rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
					>
						{pending ? "…" : "Send reset link"}
					</button>
					{error ? <ErrorBanner message={error} /> : null}
					<a
						href={signInUrl}
						className="block text-center text-xs text-[var(--pylon-ink-2,#52525b)] hover:underline"
					>
						Back to sign in
					</a>
				</form>
			)}
		</Card>
	);
}

// ---------------------------------------------------------------------------
// <ResetPassword />
// ---------------------------------------------------------------------------

export interface ResetPasswordProps {
	/** Token from the reset email. Falls back to `?token=` in URL. */
	token?: string;
	/** Where to send the user after a successful reset. Default: `/sign-in`. */
	afterResetUrl?: string;
	title?: ReactNode;
	className?: string;
}

export function ResetPassword({
	token: tokenProp,
	afterResetUrl = "/sign-in",
	title = "Choose a new password",
	className,
}: ResetPasswordProps) {
	const [token, setToken] = useState<string | null>(null);
	const [next, setNext] = useState("");
	const [confirm, setConfirm] = useState("");
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [done, setDone] = useState(false);

	useEffect(() => {
		if (tokenProp) {
			setToken(tokenProp);
			return;
		}
		if (typeof window !== "undefined") {
			const t = new URLSearchParams(window.location.search).get("token");
			setToken(t);
		}
	}, [tokenProp]);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setError(null);
		if (!token) {
			setError("Missing reset token. Use the link from your email.");
			return;
		}
		if (next !== confirm) {
			setError("Passwords don't match.");
			return;
		}
		setPending(true);
		try {
			await completePasswordReset({ token, newPassword: next });
			setDone(true);
			if (typeof window !== "undefined") {
				window.location.assign(afterResetUrl);
			}
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<Card className={className}>
			<Heading title={title} />
			{done ? (
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					Password updated. Redirecting…
				</p>
			) : (
				<form onSubmit={onSubmit} className="space-y-3">
					<Field
						label="New password"
						type="password"
						value={next}
						onChange={setNext}
						required
						autoComplete="new-password"
					/>
					<Field
						label="Confirm"
						type="password"
						value={confirm}
						onChange={setConfirm}
						required
						autoComplete="new-password"
					/>
					<button
						type="submit"
						disabled={pending || !token}
						className="w-full rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
					>
						{pending ? "…" : "Update password"}
					</button>
					{error ? <ErrorBanner message={error} /> : null}
				</form>
			)}
		</Card>
	);
}

// ---------------------------------------------------------------------------
// Shared chrome (duplicated thinly from SignIn to keep this file self-contained)
// ---------------------------------------------------------------------------

function Card({
	className,
	children,
}: {
	className?: string;
	children: ReactNode;
}) {
	return (
		<div
			className={cn(
				"mx-auto w-full max-w-sm space-y-5 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-7 shadow-sm",
				className,
			)}
		>
			{children}
		</div>
	);
}

function Heading({ title }: { title: ReactNode }) {
	return (
		<h2 className="text-center text-lg font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
			{title}
		</h2>
	);
}

function Field({
	label,
	value,
	onChange,
	type = "text",
	required,
	autoComplete,
	placeholder,
}: {
	label: string;
	value: string;
	onChange: (v: string) => void;
	type?: string;
	required?: boolean;
	autoComplete?: string;
	placeholder?: string;
}) {
	return (
		<label className="block space-y-1.5">
			<span className="text-xs font-medium text-[var(--pylon-ink-2,#52525b)]">
				{label}
			</span>
			<input
				type={type}
				value={value}
				onChange={(e) => onChange(e.target.value)}
				required={required}
				autoComplete={autoComplete}
				placeholder={placeholder}
				className="w-full rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
			/>
		</label>
	);
}

function ErrorBanner({ message }: { message: string }) {
	return (
		<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
			{message}
		</p>
	);
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "MISSING_EMAIL":
				return "Enter your email.";
			case "RATE_LIMITED":
				return "Too many reset requests. Try again in a minute.";
			case "INVALID_TOKEN":
				return "This reset link is invalid or expired. Request a new one.";
			case "WEAK_PASSWORD":
				return "Pick a stronger password.";
			case "PWNED_PASSWORD":
				return "This password has been seen in known breaches. Pick another.";
			case "USER_NOT_FOUND":
				return "Account no longer exists.";
			case "MISSING_FIELD":
				return "Token and new password are both required.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong.";
}
