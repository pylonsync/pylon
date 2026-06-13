"use client";
import React from "react";

import {
	type FormEvent,
	type ReactNode,
	useCallback,
	useEffect,
	useState,
} from "react";
import { useAuth } from "../hooks/useAuth";
import {
	type ActiveSession,
	type ApiKeyCreated,
	ApiError,
	type ApiKeySummary,
	changePassword,
	createApiKey,
	listActiveSessions,
	listApiKeys,
	revokeAllSessions,
	revokeApiKey,
} from "../lib/api";
import { cn } from "../lib/cn";

export interface UserProfileProps {
	/** Hide sections you don't need. */
	hidePassword?: boolean;
	hideSessions?: boolean;
	hideApiKeys?: boolean;
	className?: string;
}

/**
 * Drop-in account management surface — Clerk's `<UserProfile />` shape.
 * Sections:
 *   - Identity card (user_id, active org)
 *   - Password change (skipped for OAuth-only accounts)
 *   - Active sessions + "Sign out other devices"
 *   - API keys (create / list / revoke)
 *
 * Each section can be hidden via props if the consuming app handles it
 * elsewhere. Renders nothing for signed-out users.
 */
export function UserProfile({
	hidePassword,
	hideSessions,
	hideApiKeys,
	className,
}: UserProfileProps) {
	const { isSignedIn, userId, tenantId } = useAuth();
	if (!isSignedIn) return null;
	return (
		<div
			className={cn(
				"mx-auto w-full max-w-xl space-y-6 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-7 shadow-sm",
				className,
			)}
		>
			<IdentityCard userId={userId} tenantId={tenantId} />
			{!hidePassword ? <PasswordChange /> : null}
			{!hideSessions ? <Sessions /> : null}
			{!hideApiKeys ? <ApiKeys /> : null}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

function IdentityCard({
	userId,
	tenantId,
}: {
	userId: string | null;
	tenantId: string | null;
}) {
	return (
		<section className="space-y-1">
			<SectionLabel>Account</SectionLabel>
			<div className="rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper-2,#f4f4f5)] p-3">
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">User ID</p>
				<p className="break-all font-mono text-sm text-[var(--pylon-ink,#0a0a0a)]">
					{userId ?? "—"}
				</p>
				{tenantId ? (
					<>
						<p className="mt-2 text-xs text-[var(--pylon-ink-3,#71717a)]">
							Active org
						</p>
						<p className="break-all font-mono text-sm text-[var(--pylon-ink,#0a0a0a)]">
							{tenantId}
						</p>
					</>
				) : null}
			</div>
		</section>
	);
}

// ---------------------------------------------------------------------------
// Password change
// ---------------------------------------------------------------------------

function PasswordChange() {
	const [current, setCurrent] = useState("");
	const [next, setNext] = useState("");
	const [confirm, setConfirm] = useState("");
	const [pending, setPending] = useState(false);
	const [status, setStatus] =
		useState<{ ok: boolean; message: string } | null>(null);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setStatus(null);
		if (next !== confirm) {
			setStatus({ ok: false, message: "New passwords don't match." });
			return;
		}
		setPending(true);
		try {
			await changePassword({ currentPassword: current, newPassword: next });
			setStatus({ ok: true, message: "Password updated." });
			setCurrent("");
			setNext("");
			setConfirm("");
		} catch (err) {
			setStatus({ ok: false, message: messageFromError(err) });
		} finally {
			setPending(false);
		}
	}

	return (
		<section className="space-y-3">
			<SectionLabel>Change password</SectionLabel>
			<form onSubmit={onSubmit} className="space-y-2">
				<PasswordField
					label="Current"
					value={current}
					onChange={setCurrent}
					autoComplete="current-password"
				/>
				<PasswordField
					label="New"
					value={next}
					onChange={setNext}
					autoComplete="new-password"
				/>
				<PasswordField
					label="Confirm"
					value={confirm}
					onChange={setConfirm}
					autoComplete="new-password"
				/>
				<button
					type="submit"
					disabled={pending || !current || !next}
					className="w-full rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
				>
					{pending ? "…" : "Update password"}
				</button>
				{status ? (
					<p
						className={cn(
							"rounded-md border px-3 py-2 text-xs",
							status.ok
								? "border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper-2,#f4f4f5)] text-[var(--pylon-ink-2,#52525b)]"
								: "border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] text-[var(--pylon-error-ink,#b91c1c)]",
						)}
					>
						{status.message}
					</p>
				) : null}
			</form>
		</section>
	);
}

function PasswordField({
	label,
	value,
	onChange,
	autoComplete,
}: {
	label: string;
	value: string;
	onChange: (v: string) => void;
	autoComplete: string;
}) {
	return (
		<label className="block space-y-1">
			<span className="text-xs font-medium text-[var(--pylon-ink-2,#52525b)]">
				{label}
			</span>
			<input
				type="password"
				value={value}
				onChange={(e) => onChange(e.target.value)}
				autoComplete={autoComplete}
				className="w-full rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
			/>
		</label>
	);
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

function Sessions() {
	const [sessions, setSessions] = useState<ActiveSession[] | null>(null);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		setSessions(await listActiveSessions());
	}, []);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	async function onSignOutOthers() {
		setError(null);
		setPending(true);
		try {
			await revokeAllSessions();
			void refresh();
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<section className="space-y-3">
			<div className="flex items-baseline justify-between gap-2">
				<SectionLabel>Active sessions</SectionLabel>
				{sessions && sessions.length > 1 ? (
					<button
						type="button"
						onClick={onSignOutOthers}
						disabled={pending}
						className="text-xs font-medium text-[var(--pylon-error-ink,#b91c1c)] hover:underline disabled:opacity-50"
					>
						Sign out all devices
					</button>
				) : null}
			</div>
			{!sessions ? (
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					Loading…
				</p>
			) : sessions.length === 0 ? (
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					No active sessions.
				</p>
			) : (
				<ul className="divide-y divide-[var(--pylon-rule,#e5e7eb)] rounded-md border border-[var(--pylon-rule,#e5e7eb)]">
					{sessions.map((s) => (
						<li
							key={s.token_prefix + s.created_at}
							className="flex items-center justify-between gap-2 px-3 py-2 text-sm"
						>
							<span className="min-w-0">
								<span className="block truncate font-medium text-[var(--pylon-ink,#0a0a0a)]">
									{s.device || "Unknown device"}
								</span>
								<span className="block text-[11px] text-[var(--pylon-ink-3,#71717a)]">
									Token …{s.token_prefix} · expires{" "}
									{formatRelative(s.expires_at)}
								</span>
							</span>
						</li>
					))}
				</ul>
			)}
			{error ? <ErrorBanner message={error} /> : null}
		</section>
	);
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

function ApiKeys() {
	const [keys, setKeys] = useState<ApiKeySummary[] | null>(null);
	const [creating, setCreating] = useState(false);
	const [newName, setNewName] = useState("");
	const [lastCreated, setLastCreated] = useState<ApiKeyCreated | null>(null);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		setKeys(await listApiKeys());
	}, []);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	async function onCreate(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			const created = await createApiKey({ name: newName.trim() });
			setLastCreated(created);
			setNewName("");
			setCreating(false);
			void refresh();
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	async function onRevoke(id: string) {
		setError(null);
		try {
			await revokeApiKey(id);
			void refresh();
		} catch (err) {
			setError(messageFromError(err));
		}
	}

	return (
		<section className="space-y-3">
			<div className="flex items-baseline justify-between gap-2">
				<SectionLabel>API keys</SectionLabel>
				{!creating ? (
					<button
						type="button"
						onClick={() => setCreating(true)}
						className="text-xs font-medium text-[var(--pylon-ink,#0a0a0a)] hover:underline"
					>
						New key
					</button>
				) : null}
			</div>
			{creating ? (
				<form onSubmit={onCreate} className="flex gap-2">
					<input
						value={newName}
						onChange={(e) => setNewName(e.target.value)}
						placeholder="Key name"
						required
						autoFocus
						className="flex-1 rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
					/>
					<button
						type="submit"
						disabled={pending || !newName.trim()}
						className="rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
					>
						{pending ? "…" : "Create"}
					</button>
					<button
						type="button"
						onClick={() => setCreating(false)}
						className="rounded-md px-3 py-2 text-xs text-[var(--pylon-ink-2,#52525b)] hover:underline"
					>
						Cancel
					</button>
				</form>
			) : null}
			{lastCreated ? (
				<div className="space-y-1.5 rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper-2,#f4f4f5)] p-3">
					<p className="text-[11px] font-medium uppercase tracking-wider text-[var(--pylon-ink-2,#52525b)]">
						Copy now — won't be shown again
					</p>
					<code className="block break-all rounded bg-[var(--pylon-paper,#ffffff)] px-2 py-1 font-mono text-xs text-[var(--pylon-ink,#0a0a0a)]">
						{lastCreated.key}
					</code>
					<button
						type="button"
						onClick={() => setLastCreated(null)}
						className="text-[11px] text-[var(--pylon-ink-3,#71717a)] hover:underline"
					>
						Dismiss
					</button>
				</div>
			) : null}
			{!keys ? (
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					Loading…
				</p>
			) : keys.length === 0 ? (
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					No keys yet.
				</p>
			) : (
				<ul className="divide-y divide-[var(--pylon-rule,#e5e7eb)] rounded-md border border-[var(--pylon-rule,#e5e7eb)]">
					{keys.map((k) => (
						<li
							key={k.id}
							className="flex items-center justify-between gap-2 px-3 py-2 text-sm"
						>
							<span className="min-w-0">
								<span className="block truncate font-medium text-[var(--pylon-ink,#0a0a0a)]">
									{k.name}
								</span>
								<span className="block text-[11px] text-[var(--pylon-ink-3,#71717a)]">
									{k.prefix}… · last used{" "}
									{k.last_used_at ? formatRelative(k.last_used_at) : "never"}
								</span>
							</span>
							<button
								type="button"
								onClick={() => onRevoke(k.id)}
								className="rounded-md px-2 py-1 text-xs text-[var(--pylon-error-ink,#b91c1c)] transition-colors hover:bg-[var(--pylon-error-bg,#fef2f2)]"
							>
								Revoke
							</button>
						</li>
					))}
				</ul>
			)}
			{error ? <ErrorBanner message={error} /> : null}
		</section>
	);
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

function SectionLabel({ children }: { children: ReactNode }) {
	return (
		<p className="text-xs font-medium uppercase tracking-wider text-[var(--pylon-ink-2,#52525b)]">
			{children}
		</p>
	);
}

function ErrorBanner({ message }: { message: string }) {
	return (
		<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
			{message}
		</p>
	);
}

function formatRelative(unixSecs: number): string {
	if (!unixSecs) return "—";
	const ms = unixSecs * 1000;
	const diff = ms - Date.now();
	const abs = Math.abs(diff);
	const day = 86_400_000;
	const hour = 3_600_000;
	const min = 60_000;
	const sign = diff < 0 ? "ago" : "from now";
	if (abs > day) return `${Math.round(abs / day)}d ${sign}`;
	if (abs > hour) return `${Math.round(abs / hour)}h ${sign}`;
	if (abs > min) return `${Math.round(abs / min)}m ${sign}`;
	return diff < 0 ? "just now" : "soon";
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "INVALID_CREDENTIALS":
			case "BAD_CURRENT_PASSWORD":
				return "Current password is wrong.";
			case "WEAK_PASSWORD":
				return "Pick a stronger password.";
			case "MISSING_PASSWORD":
				return "Enter a new password.";
			case "API_KEY_AUTH_FORBIDDEN":
				return "This action needs a session, not an API key.";
			case "AUTH_REQUIRED":
				return "Sign in to manage your account.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong.";
}
