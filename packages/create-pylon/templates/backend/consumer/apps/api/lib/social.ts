import type { MutationCtx, QueryCtx } from "@pylonsync/functions";

// Shared checks for the functions in functions/.

export const CAPTION_MAX = 2200;
export const COMMENT_MAX = 500;
export const BIO_MAX = 150;
export const HANDLE_PATTERN = /^[a-z0-9._]{2,24}$/;

/**
 * A post or avatar image must be a demo photo served from public/images or
 * a file uploaded to this server (`/api/files/<id>`). Anything else is
 * refused, so a client cannot point the feed at an arbitrary host.
 */
export function isImageUrl(url: string): boolean {
	if (url.includes("..") || url.includes("//")) return false;
	return /^\/images\/[a-z0-9/_.-]+\.(jpg|jpeg|png|webp)$/i.test(url) ||
		/^\/api\/files\/[A-Za-z0-9_.-]+$/.test(url);
}

/** The caller's Profile row, or null when they have not made one yet. */
export async function findProfile(
	ctx: Pick<QueryCtx, "db"> | Pick<MutationCtx, "db">,
	userId: string,
): Promise<ProfileRow | null> {
	const rows = await ctx.db.query("Profile", { userId, $limit: 1 });
	return (rows[0] as unknown as ProfileRow | undefined) ?? null;
}

/** The caller's Profile row. Throws NO_PROFILE when there is none. */
export async function requireProfile(
	ctx: Pick<MutationCtx, "db" | "error">,
	userId: string,
): Promise<ProfileRow> {
	const profile = await findProfile(ctx, userId);
	if (!profile) {
		throw ctx.error("NO_PROFILE", "Create your profile first.");
	}
	return profile;
}

export interface ProfileRow {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	avatarUrl?: string | null;
	createdAt: string;
}
