import { pylon } from "@/lib/pylon";
import { Feed } from "./components/Feed";

export const dynamic = "force-dynamic";

type FeedItem = {
	id: string;
	body: string;
	createdAt: string;
	author: { id: string; handle: string; displayName: string } | null;
	likeCount: number;
	likedByMe: boolean;
};

type Profile = {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	createdAt: string;
};

export default async function HomePage() {
	const [feed, me] = await Promise.all([
		pylon
			.json<FeedItem[]>("/api/fn/feed", {
				method: "POST",
				body: "{}",
				headers: { "Content-Type": "application/json" },
			})
			.catch(() => [] as FeedItem[]),
		pylon
			.json<Profile | null>("/api/fn/myProfile", {
				method: "POST",
				body: "{}",
				headers: { "Content-Type": "application/json" },
			})
			.catch(() => null),
	]);

	return (
		<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
			<header className="space-y-2">
				<h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
				<p className="text-sm text-neutral-500 dark:text-neutral-400">
					Public feed scaffold. Profile, Post, Like — wide-open reads, owner-
					only writes (enforced by Pylon row-level policies).
				</p>
			</header>

			<Feed initialFeed={feed} initialMe={me} />
		</main>
	);
}
