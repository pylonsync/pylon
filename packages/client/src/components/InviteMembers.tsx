"use client";

import {
	type FormEvent,
	type ReactNode,
	useCallback,
	useEffect,
	useState,
} from "react";
import { useAuth } from "../hooks/useAuth";
import {
	ApiError,
	createInvite,
	type InviteResult,
	listInvites,
	listOrgMembers,
	type OrgMember,
	type PendingInvite,
	removeMember,
	revokeInvite,
	updateMemberRole,
} from "../lib/api";
import { cn } from "../lib/cn";

export interface InviteMembersProps {
	/** Org to manage. Defaults to the user's active tenant. */
	orgId?: string;
	/** Roles offered in the role picker. Default: ["member", "admin"]. */
	roles?: string[];
	/** Hide the members roster (focus solely on the invite form + pending list). */
	hideMembers?: boolean;
	/** Hide the pending invites list. */
	hidePending?: boolean;
	/** Optional callback after a new invite is sent. */
	onInvited?: (invite: InviteResult) => void;
	className?: string;
}

/**
 * Drop-in members + invites surface. Owner / admin only — non-managers
 * see a forbidden notice. Composes three concerns:
 *
 *   - Invite form (POST /api/auth/orgs/:id/invites)
 *   - Pending invites with revoke (GET + DELETE on the same path)
 *   - Member roster with role change + remove (GET/PUT/DELETE on members)
 *
 * For a focused subset, set `hideMembers` or `hidePending`.
 */
export function InviteMembers({
	orgId: orgIdOverride,
	roles = ["member", "admin"],
	hideMembers,
	hidePending,
	onInvited,
	className,
}: InviteMembersProps) {
	const { isSignedIn, tenantId, userId, session } = useAuth();
	const orgId = orgIdOverride ?? tenantId;

	const [members, setMembers] = useState<OrgMember[] | null>(null);
	const [invites, setInvites] = useState<PendingInvite[] | null>(null);
	const [loadError, setLoadError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		if (!orgId) return;
		setLoadError(null);
		try {
			const [m, i] = await Promise.all([
				listOrgMembers(orgId),
				listInvites(orgId),
			]);
			setMembers(m);
			setInvites(i);
		} catch (err) {
			setLoadError(messageFromError(err));
		}
	}, [orgId]);

	useEffect(() => {
		if (!isSignedIn || !orgId) return;
		void refresh();
	}, [isSignedIn, orgId, refresh]);

	if (!isSignedIn) return null;

	if (!orgId) {
		return (
			<EmptyShell className={className}>
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					Select an organization to manage members.
				</p>
			</EmptyShell>
		);
	}

	// Manager role gating mirrors what the server enforces — keeps the
	// UI honest instead of showing inputs that 403 on submit.
	const callerRoles = session?.roles ?? [];
	const canManage =
		callerRoles.includes("owner") || callerRoles.includes("admin");
	if (!canManage) {
		return (
			<EmptyShell className={className}>
				<p className="text-sm text-[var(--pylon-ink-2,#52525b)]">
					You need owner or admin role to invite members.
				</p>
			</EmptyShell>
		);
	}

	return (
		<div
			className={cn(
				"mx-auto w-full max-w-xl space-y-6 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-6 shadow-sm",
				className,
			)}
		>
			<InviteForm
				orgId={orgId}
				roles={roles}
				onInvited={(inv) => {
					onInvited?.(inv);
					void refresh();
				}}
			/>

			{!hidePending ? (
				<PendingList
					invites={invites}
					orgId={orgId}
					onRevoked={refresh}
				/>
			) : null}

			{!hideMembers ? (
				<MemberRoster
					members={members}
					orgId={orgId}
					roles={roles}
					currentUserId={userId}
					currentUserRoles={callerRoles}
					onChanged={refresh}
				/>
			) : null}

			{loadError ? <ErrorBanner message={loadError} /> : null}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Invite form
// ---------------------------------------------------------------------------

function InviteForm({
	orgId,
	roles,
	onInvited,
}: {
	orgId: string;
	roles: string[];
	onInvited: (invite: InviteResult) => void;
}) {
	const [email, setEmail] = useState("");
	const [role, setRole] = useState(roles[0] ?? "member");
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [lastInvite, setLastInvite] = useState<InviteResult | null>(null);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			const invite = await createInvite(orgId, email.trim(), role);
			setEmail("");
			setLastInvite(invite);
			onInvited(invite);
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<div className="space-y-3">
			<SectionLabel>Invite by email</SectionLabel>
			<form onSubmit={onSubmit} className="flex gap-2">
				<input
					type="email"
					value={email}
					onChange={(e) => setEmail(e.target.value)}
					required
					placeholder="teammate@acme.com"
					className="flex-1 rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
				/>
				<select
					value={role}
					onChange={(e) => setRole(e.target.value)}
					className="rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-2 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
				>
					{roles.map((r) => (
						<option key={r} value={r}>
							{capitalize(r)}
						</option>
					))}
				</select>
				<button
					type="submit"
					disabled={pending}
					className="rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
				>
					{pending ? "…" : "Invite"}
				</button>
			</form>
			{error ? <ErrorBanner message={error} /> : null}
			{lastInvite?.token ? (
				<div className="rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper-2,#f4f4f5)] p-3 text-xs text-[var(--pylon-ink-2,#52525b)]">
					<p className="mb-1.5 font-medium text-[var(--pylon-ink,#0a0a0a)]">
						Dev mode — share this link manually
					</p>
					<code className="block break-all rounded bg-[var(--pylon-paper,#ffffff)] px-2 py-1 text-[var(--pylon-ink,#0a0a0a)]">
						{lastInvite.accept_url}
					</code>
				</div>
			) : null}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Pending invites
// ---------------------------------------------------------------------------

function PendingList({
	invites,
	orgId,
	onRevoked,
}: {
	invites: PendingInvite[] | null;
	orgId: string;
	onRevoked: () => void;
}) {
	if (!invites) {
		return <SectionLabel muted>Loading pending invites…</SectionLabel>;
	}
	if (invites.length === 0) {
		return (
			<div className="space-y-2">
				<SectionLabel>Pending invites</SectionLabel>
				<p className="text-xs text-[var(--pylon-ink-3,#71717a)]">
					No invites waiting.
				</p>
			</div>
		);
	}
	return (
		<div className="space-y-2">
			<SectionLabel>Pending invites</SectionLabel>
			<ul className="divide-y divide-[var(--pylon-rule,#e5e7eb)] rounded-md border border-[var(--pylon-rule,#e5e7eb)]">
				{invites.map((inv) => (
					<InviteRow
						key={inv.id}
						invite={inv}
						orgId={orgId}
						onRevoked={onRevoked}
					/>
				))}
			</ul>
		</div>
	);
}

function InviteRow({
	invite,
	orgId,
	onRevoked,
}: {
	invite: PendingInvite;
	orgId: string;
	onRevoked: () => void;
}) {
	const [pending, setPending] = useState(false);
	async function onRevoke() {
		setPending(true);
		try {
			await revokeInvite(orgId, invite.id);
			onRevoked();
		} finally {
			setPending(false);
		}
	}
	return (
		<li className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
			<span className="min-w-0">
				<span className="block truncate font-medium text-[var(--pylon-ink,#0a0a0a)]">
					{invite.email}
				</span>
				<span className="block text-[11px] text-[var(--pylon-ink-3,#71717a)]">
					{capitalize(invite.role)} · expires{" "}
					{formatRelative(invite.expires_at)}
				</span>
			</span>
			<button
				type="button"
				onClick={onRevoke}
				disabled={pending}
				className="rounded-md px-2 py-1 text-xs text-[var(--pylon-error-ink,#b91c1c)] transition-colors hover:bg-[var(--pylon-error-bg,#fef2f2)] disabled:opacity-50"
			>
				{pending ? "…" : "Revoke"}
			</button>
		</li>
	);
}

// ---------------------------------------------------------------------------
// Member roster
// ---------------------------------------------------------------------------

function MemberRoster({
	members,
	orgId,
	roles,
	currentUserId,
	currentUserRoles,
	onChanged,
}: {
	members: OrgMember[] | null;
	orgId: string;
	roles: string[];
	currentUserId: string | null;
	currentUserRoles: string[];
	onChanged: () => void;
}) {
	if (!members) {
		return <SectionLabel muted>Loading members…</SectionLabel>;
	}
	const isOwner = currentUserRoles.includes("owner");
	// Owners are allowed to promote/demote anyone; admins can't touch
	// owners. Pre-compute "owner picker" so the dropdown only includes
	// "owner" when the caller is allowed to assign it.
	const assignableRoles = isOwner ? [...roles, "owner"] : roles;
	const deduped = Array.from(new Set(assignableRoles));

	return (
		<div className="space-y-2">
			<SectionLabel>Members</SectionLabel>
			<ul className="divide-y divide-[var(--pylon-rule,#e5e7eb)] rounded-md border border-[var(--pylon-rule,#e5e7eb)]">
				{members.map((m) => (
					<MemberRow
						key={m.user_id}
						member={m}
						orgId={orgId}
						roles={deduped}
						isSelf={m.user_id === currentUserId}
						canManageRoles={
							isOwner || (currentUserRoles.includes("admin") && m.role !== "owner")
						}
						onChanged={onChanged}
					/>
				))}
			</ul>
		</div>
	);
}

function MemberRow({
	member,
	orgId,
	roles,
	isSelf,
	canManageRoles,
	onChanged,
}: {
	member: OrgMember;
	orgId: string;
	roles: string[];
	isSelf: boolean;
	canManageRoles: boolean;
	onChanged: () => void;
}) {
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function onRoleChange(role: string) {
		setError(null);
		setPending(true);
		try {
			await updateMemberRole(orgId, member.user_id, role);
			onChanged();
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	async function onRemove() {
		setError(null);
		setPending(true);
		try {
			await removeMember(orgId, member.user_id);
			onChanged();
		} catch (err) {
			setError(messageFromError(err));
		} finally {
			setPending(false);
		}
	}

	return (
		<li className="flex flex-col gap-2 px-3 py-2 text-sm sm:flex-row sm:items-center sm:justify-between">
			<span className="min-w-0">
				<span className="block truncate font-medium text-[var(--pylon-ink,#0a0a0a)]">
					{member.user_id}
					{isSelf ? (
						<span className="ml-1 text-[10px] uppercase tracking-wider text-[var(--pylon-ink-3,#71717a)]">
							you
						</span>
					) : null}
				</span>
				<span className="block text-[11px] text-[var(--pylon-ink-3,#71717a)]">
					Joined {formatRelative(member.joined_at)}
				</span>
			</span>
			<div className="flex items-center gap-2">
				{canManageRoles ? (
					<select
						value={member.role}
						onChange={(e) => onRoleChange(e.target.value)}
						disabled={pending}
						className="rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-2 py-1 text-xs text-[var(--pylon-ink,#0a0a0a)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
					>
						{roles.map((r) => (
							<option key={r} value={r}>
								{capitalize(r)}
							</option>
						))}
					</select>
				) : (
					<span className="rounded-md border border-[var(--pylon-rule,#e5e7eb)] px-2 py-0.5 text-[11px] text-[var(--pylon-ink-2,#52525b)]">
						{capitalize(member.role)}
					</span>
				)}
				{!isSelf && canManageRoles ? (
					<button
						type="button"
						onClick={onRemove}
						disabled={pending}
						className="rounded-md px-2 py-1 text-xs text-[var(--pylon-error-ink,#b91c1c)] transition-colors hover:bg-[var(--pylon-error-bg,#fef2f2)] disabled:opacity-50"
					>
						Remove
					</button>
				) : null}
			</div>
			{error ? (
				<p className="basis-full text-[11px] text-[var(--pylon-error-ink,#b91c1c)]">
					{error}
				</p>
			) : null}
		</li>
	);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function EmptyShell({
	children,
	className,
}: {
	children: ReactNode;
	className?: string;
}) {
	return (
		<div
			className={cn(
				"mx-auto w-full max-w-xl rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-6 shadow-sm",
				className,
			)}
		>
			{children}
		</div>
	);
}

function SectionLabel({
	children,
	muted,
}: {
	children: ReactNode;
	muted?: boolean;
}) {
	return (
		<p
			className={cn(
				"text-xs font-medium uppercase tracking-wider",
				muted
					? "text-[var(--pylon-ink-3,#71717a)]"
					: "text-[var(--pylon-ink-2,#52525b)]",
			)}
		>
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

function capitalize(s: string): string {
	if (!s) return "";
	return s.charAt(0).toUpperCase() + s.slice(1);
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
			case "INVALID_EMAIL":
				return "Enter a valid email address.";
			case "BAD_ROLE":
				return "Role must be owner, admin, or member.";
			case "LAST_OWNER":
				return "Can't demote or remove the last owner. Promote someone else first.";
			case "FORBIDDEN":
				return "You don't have permission to do that.";
			case "NOT_A_MEMBER":
				return "That user isn't a member of this org.";
			case "INVITE_CREATE_FAILED":
				return "Couldn't create invite — try again.";
			case "API_KEY_AUTH_FORBIDDEN":
				return "Member management needs a session, not an API key.";
			default:
				return err.message;
		}
	}
	if (err instanceof Error) return err.message;
	return "Something went wrong.";
}
