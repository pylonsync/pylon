"use client";
import React from "react";

import {
	type FormEvent,
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { useAuth } from "../hooks/useAuth";
import {
	ApiError,
	createOrg,
	listOrgs,
	type OrgSummary,
} from "../lib/api";
import { cn } from "../lib/cn";

export interface OrganizationSwitcherProps {
	/** Hide the "Personal account" entry (no active org). Default: visible. */
	hidePersonal?: boolean;
	/** Disable the "Create organization" menu entry. */
	hideCreate?: boolean;
	/** Called after a switch lands. Useful for analytics / route changes. */
	onSwitched?: (orgId: string | null) => void;
	/** Called after a new org is created (post-creation, pre-switch). */
	onCreated?: (org: OrgSummary) => void | Promise<void>;
	/**
	 * Active org name resolved on the server (e.g. `serverData.get("Org", …)`
	 * in an SSR layout). Seeds the trigger label so it renders the real name on
	 * the first paint — no "Personal" → real-name flash, and no pop-in waiting
	 * for `useAuth()`/`listOrgs()` to hydrate. The dropdown list still loads
	 * client-side, but it's hidden until opened.
	 */
	initialActiveName?: string;
	className?: string;
}

/**
 * Drop-in org switcher. Lists the signed-in user's orgs from
 * `/api/auth/orgs`, lets them switch via `selectOrg` from useSession,
 * and inline-creates a new org via `/api/auth/orgs` POST.
 *
 * Renders nothing for signed-out users — wrap in <SignedIn> if you
 * need that. Apps that don't use the framework's built-in Org / OrgMember
 * entities will see an empty list (and just the create form, if enabled).
 */
export function OrganizationSwitcher({
	hidePersonal,
	hideCreate,
	onSwitched,
	onCreated,
	initialActiveName,
	className,
}: OrganizationSwitcherProps) {
	const { isSignedIn, tenantId, selectOrg, clearOrg } = useAuth();
	const [open, setOpen] = useState(false);
	const [mode, setMode] = useState<"list" | "create">("list");
	const [orgs, setOrgs] = useState<OrgSummary[] | null>(null);
	const [switching, setSwitching] = useState(false);
	const rootRef = useRef<HTMLDivElement | null>(null);

	const refresh = useCallback(async () => {
		const next = await listOrgs();
		setOrgs(next);
	}, []);

	useEffect(() => {
		if (!isSignedIn) {
			setOrgs(null);
			return;
		}
		void refresh();
	}, [isSignedIn, refresh]);

	useEffect(() => {
		if (!open) return;
		function onDocClick(e: MouseEvent) {
			if (!rootRef.current?.contains(e.target as Node)) {
				setOpen(false);
				setMode("list");
			}
		}
		function onKey(e: KeyboardEvent) {
			if (e.key === "Escape") {
				setOpen(false);
				setMode("list");
			}
		}
		document.addEventListener("mousedown", onDocClick);
		document.addEventListener("keydown", onKey);
		return () => {
			document.removeEventListener("mousedown", onDocClick);
			document.removeEventListener("keydown", onKey);
		};
	}, [open]);

	// A server-seeded active name means the SSR layout already rendered this in
	// an authenticated shell — render immediately instead of waiting for
	// `isSignedIn` to hydrate (which causes a pop-in).
	if (!isSignedIn && !initialActiveName) return null;

	const active = orgs?.find((o) => o.id === tenantId) ?? null;
	const label =
		active?.name ?? initialActiveName ?? (hidePersonal ? "Select org" : "Personal");

	async function switchTo(orgId: string | null) {
		setSwitching(true);
		try {
			if (orgId === null) {
				await clearOrg();
			} else {
				await selectOrg(orgId);
			}
			onSwitched?.(orgId);
			setOpen(false);
		} catch (err) {
			// Surface "not a member" / other server errors in console for
			// now — full toast UI is the consumer app's call.
			console.error("[pylon] selectOrg failed:", err);
		} finally {
			setSwitching(false);
		}
	}

	function handleCreated(org: OrgSummary) {
		onCreated?.(org);
		void refresh().then(() => switchTo(org.id));
		setMode("list");
	}

	return (
		<div ref={rootRef} className={cn("relative inline-flex", className)}>
			<button
				type="button"
				onClick={() => setOpen((o) => !o)}
				aria-haspopup="menu"
				aria-expanded={open}
				className="inline-flex items-center gap-2 rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-1.5 text-sm font-medium text-[var(--pylon-ink,#0a0a0a)] transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)]"
			>
				<span className="flex h-5 w-5 items-center justify-center rounded-sm bg-[var(--pylon-ink,#0a0a0a)] text-[10px] font-semibold text-[var(--pylon-paper,#ffffff)]">
					{glyph(label)}
				</span>
				<span className="max-w-[10rem] truncate">{label}</span>
				<svg
					aria-hidden
					width="10"
					height="10"
					viewBox="0 0 10 10"
					fill="none"
					className="opacity-60"
				>
					<path
						d="M2 4l3 3 3-3"
						stroke="currentColor"
						strokeWidth="1.5"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
				</svg>
			</button>
			{open ? (
				<div
					role="menu"
					className="absolute left-0 top-full z-50 mt-2 w-64 overflow-hidden rounded-lg border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] shadow-lg"
				>
					{mode === "list" ? (
						<>
							<div className="max-h-64 overflow-y-auto py-1">
								{!hidePersonal ? (
									<OrgRow
										name="Personal"
										subtitle="No active org"
										active={tenantId === null}
										onClick={() => switchTo(null)}
										disabled={switching}
									/>
								) : null}
								{(orgs ?? []).map((o) => (
									<OrgRow
										key={o.id}
										name={o.name}
										subtitle={roleLabel(o.role)}
										active={o.id === tenantId}
										onClick={() => switchTo(o.id)}
										disabled={switching}
									/>
								))}
								{orgs && orgs.length === 0 && hidePersonal ? (
									<p className="px-3 py-2 text-xs text-[var(--pylon-ink-3,#71717a)]">
										You're not a member of any orgs yet.
									</p>
								) : null}
							</div>
							{!hideCreate ? (
								<button
									type="button"
									onClick={() => setMode("create")}
									className="flex w-full items-center gap-2 border-t border-[var(--pylon-rule,#e5e7eb)] px-3 py-2.5 text-left text-sm font-medium text-[var(--pylon-ink,#0a0a0a)] transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)]"
								>
									<PlusGlyph />
									Create organization
								</button>
							) : null}
						</>
					) : (
						<CreateOrgInline
							onCreated={handleCreated}
							onCancel={() => setMode("list")}
						/>
					)}
				</div>
			) : null}
		</div>
	);
}

function OrgRow({
	name,
	subtitle,
	active,
	onClick,
	disabled,
}: {
	name: string;
	subtitle?: string;
	active: boolean;
	onClick: () => void;
	disabled?: boolean;
}) {
	return (
		<button
			type="button"
			role="menuitem"
			onClick={onClick}
			disabled={disabled}
			className={cn(
				"flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)] disabled:opacity-60",
				active && "bg-[var(--pylon-paper-2,#f4f4f5)]",
			)}
		>
			<span className="flex min-w-0 items-center gap-2">
				<span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sm bg-[var(--pylon-ink,#0a0a0a)] text-[10px] font-semibold text-[var(--pylon-paper,#ffffff)]">
					{glyph(name)}
				</span>
				<span className="min-w-0">
					<span className="block truncate text-sm font-medium text-[var(--pylon-ink,#0a0a0a)]">
						{name}
					</span>
					{subtitle ? (
						<span className="block truncate text-[11px] text-[var(--pylon-ink-3,#71717a)]">
							{subtitle}
						</span>
					) : null}
				</span>
			</span>
			{active ? (
				<svg
					aria-hidden
					width="14"
					height="14"
					viewBox="0 0 14 14"
					fill="none"
				>
					<path
						d="M3 7.5L6 10.5L11 4.5"
						stroke="currentColor"
						strokeWidth="1.6"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
				</svg>
			) : null}
		</button>
	);
}

function CreateOrgInline({
	onCreated,
	onCancel,
}: {
	onCreated: (org: OrgSummary) => void | Promise<void>;
	onCancel: () => void;
}) {
	return (
		<div className="p-3">
			<CreateOrganizationForm onCreated={onCreated} />
			<button
				type="button"
				onClick={onCancel}
				className="mt-2 block w-full text-center text-xs text-[var(--pylon-ink-3,#71717a)] hover:underline"
			>
				Back
			</button>
		</div>
	);
}

// ---------------------------------------------------------------------------
// CreateOrganization — standalone, usable inside a modal or page.
// ---------------------------------------------------------------------------

export interface CreateOrganizationProps {
	onCreated?: (org: OrgSummary) => void | Promise<void>;
	/** Title shown above the form. */
	title?: ReactNode;
	className?: string;
}

export function CreateOrganization({
	onCreated,
	title = "Create organization",
	className,
}: CreateOrganizationProps) {
	return (
		<div
			className={cn(
				"mx-auto w-full max-w-sm space-y-4 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-6 shadow-sm",
				className,
			)}
		>
			<h2 className="text-base font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
				{title}
			</h2>
			<CreateOrganizationForm onCreated={onCreated} />
		</div>
	);
}

function CreateOrganizationForm({
	onCreated,
}: {
	onCreated?: (org: OrgSummary) => void | Promise<void>;
}) {
	const [name, setName] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(false);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		const trimmed = name.trim();
		if (!trimmed) {
			setError("Pick a name for your org.");
			return;
		}
		setError(null);
		setPending(true);
		try {
			const org = await createOrg(trimmed);
			setName("");
			// Await the callback so anything it does (selectOrg, navigation,
			// step advance) that rejects surfaces here as an error instead of
			// an unhandled rejection that leaves the form looking inert.
			await onCreated?.(org);
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<form onSubmit={onSubmit} className="space-y-2.5">
			<label className="block space-y-1.5">
				<span className="text-xs font-medium text-[var(--pylon-ink-2,#52525b)]">
					Organization name
				</span>
				<input
					value={name}
					onChange={(e) => setName(e.target.value)}
					placeholder="Acme Inc."
					autoFocus
					className="w-full rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
				/>
			</label>
			<button
				type="submit"
				disabled={pending}
				className="w-full rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
			>
				{pending ? "…" : "Create"}
			</button>
			{error ? (
				<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
					{error}
				</p>
			) : null}
		</form>
	);
}

function PlusGlyph() {
	return (
		<svg aria-hidden width="14" height="14" viewBox="0 0 14 14" fill="none">
			<path
				d="M7 3v8M3 7h8"
				stroke="currentColor"
				strokeWidth="1.6"
				strokeLinecap="round"
			/>
		</svg>
	);
}

function glyph(name: string): string {
	const parts = name.split(/\s+/).filter(Boolean);
	if (parts.length >= 2) return (parts[0]![0]! + parts[1]![0]!).toUpperCase();
	return (parts[0] ?? name).slice(0, 1).toUpperCase();
}

function roleLabel(role: string): string {
	if (!role) return "";
	return role.charAt(0).toUpperCase() + role.slice(1);
}

function messageFromError(err: unknown): string {
	if (err instanceof ApiError) {
		switch (err.code) {
			case "MISSING_NAME":
				return "Pick a name for your org.";
			case "ORG_CREATE_FAILED":
				return "Couldn't create org. Make sure Org / OrgMember entities are declared in your manifest.";
			case "API_KEY_AUTH_FORBIDDEN":
				return "Org management requires a real session — not an API key.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong.";
}
