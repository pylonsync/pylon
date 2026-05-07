"use client";

import { useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { Button, Input, Card, CardHeader, CardContent } from "@__APP_NAME_KEBAB__/ui";

type Post = {
	id: string;
	authorId: string;
	body: string;
	createdAt: string;
};

type Profile = {
	id: string;
	userId: string;
	handle: string;
	displayName: string;
	bio?: string | null;
	createdAt: string;
};

type Like = {
	id: string;
	postId: string;
	profileId: string;
	createdAt: string;
};

export function Feed() {
	// Three live subscriptions. The sync engine pushes diffs over
	// WebSocket so a new Post / Like from any other client renders
	// here within ~ms. Joining client-side keeps the schema simple
	// — for very large feeds, a server-side aggregated query is
	// cheaper, but the live story is more important to demonstrate.
	const { data: posts = [] } = db.useQuery<Post>("Post", {
		orderBy: { createdAt: "desc" },
		limit: 100,
	});
	const { data: profiles = [] } = db.useQuery<Profile>("Profile", {});
	const { data: likes = [] } = db.useQuery<Like>("Like", {});
	const { data: myProfileRows = [] } = db.useQuery<Profile>("Profile", {});

	const profilesById = useMemo(() => {
		const map = new Map<string, Profile>();
		for (const p of profiles) map.set(p.id, p);
		return map;
	}, [profiles]);

	const myProfile = useMemo(() => {
		// `myProfile` server-side filters by auth.userId; from the
		// client we don't have userId here without a separate auth
		// fetch, so we lean on the Profile policy: every Profile
		// row in the local store IS visible (public reads). To find
		// "ours" we'd want auth.userId; the scaffold uses a
		// localStorage-cached selection instead.
		if (typeof window === "undefined") return null;
		const id = window.localStorage.getItem("__APP_NAME_SNAKE___profile_id");
		if (!id) return null;
		return myProfileRows.find((p) => p.id === id) ?? null;
	}, [myProfileRows]);

	if (!myProfile) {
		return (
			<ProfileSetup
				existingHandles={profiles.map((p) => p.handle)}
				onSaved={(p) => {
					window.localStorage.setItem("__APP_NAME_SNAKE___profile_id", p.id);
					// Force re-render by reading from the live profiles array.
				}}
			/>
		);
	}

	const items = posts.map((post) => {
		const author = profilesById.get(post.authorId) ?? null;
		const postLikes = likes.filter((l) => l.postId === post.id);
		const likedByMe = postLikes.some((l) => l.profileId === myProfile.id);
		return { post, author, likeCount: postLikes.length, likedByMe };
	});

	return <FeedView me={myProfile} items={items} />;
}

function FeedView({
	me,
	items,
}: {
	me: Profile;
	items: Array<{
		post: Post;
		author: Profile | null;
		likeCount: number;
		likedByMe: boolean;
	}>;
}) {
	const [body, setBody] = useState("");
	const [posting, setPosting] = useState(false);

	async function post() {
		const trimmed = body.trim();
		if (!trimmed) return;
		setBody("");
		setPosting(true);
		try {
			await callFn("createPost", { body: trimmed });
		} finally {
			setPosting(false);
		}
	}

	async function toggleLike(postId: string) {
		try {
			await callFn("toggleLike", { postId });
		} catch (e) {
			window.alert(`Like failed: ${String(e)}`);
		}
	}

	async function remove(postId: string) {
		try {
			await callFn("deletePost", { id: postId });
		} catch (e) {
			window.alert(`Delete failed: ${String(e)}`);
		}
	}

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<div>
							<div className="text-sm font-medium">{me.displayName}</div>
							<div className="text-xs text-neutral-400 font-mono">
								@{me.handle}
							</div>
						</div>
					</div>
				</CardHeader>
				<CardContent>
					<form
						onSubmit={(e) => {
							e.preventDefault();
							post();
						}}
						className="space-y-2"
					>
						<textarea
							value={body}
							onChange={(e) => setBody(e.target.value)}
							placeholder={`What's on your mind, ${me.displayName.split(" ")[0]}?`}
							rows={3}
							maxLength={1000}
							disabled={posting}
							className="w-full rounded-md border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm placeholder:text-neutral-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 resize-none"
						/>
						<div className="flex items-center justify-between">
							<span className="text-xs text-neutral-400">
								{body.length}/1000
							</span>
							<Button
								type="submit"
								variant="primary"
								disabled={posting || !body.trim()}
								size="sm"
							>
								Post
							</Button>
						</div>
					</form>
				</CardContent>
			</Card>

			{items.length === 0 ? (
				<p className="text-sm text-neutral-500 text-center py-8">
					No posts yet. Be the first.
				</p>
			) : (
				<ul className="space-y-3">
					{items.map(({ post, author, likeCount, likedByMe }) => (
						<li key={post.id}>
							<Card>
								<CardContent className="space-y-3">
									<div className="flex items-baseline justify-between">
										<div className="text-sm">
											<span className="font-medium">
												{author?.displayName ?? "Unknown"}
											</span>{" "}
											<span className="text-neutral-400 font-mono text-xs">
												@{author?.handle ?? "?"}
											</span>
										</div>
										<span className="text-xs text-neutral-400">
											{new Date(post.createdAt).toLocaleString(undefined, {
												month: "short",
												day: "numeric",
												hour: "numeric",
												minute: "2-digit",
											})}
										</span>
									</div>
									<p className="text-sm whitespace-pre-wrap break-words">
										{post.body}
									</p>
									<div className="flex items-center gap-2">
										<button
											onClick={() => toggleLike(post.id)}
											className={`text-xs font-mono px-2 py-1 rounded border transition-colors ${
												likedByMe
													? "border-pink-300 dark:border-pink-700 text-pink-600 dark:text-pink-300 bg-pink-50 dark:bg-pink-950"
													: "border-neutral-200 dark:border-neutral-800 text-neutral-500 hover:bg-neutral-50 dark:hover:bg-neutral-900"
											}`}
										>
											{likedByMe ? "♥" : "♡"} {likeCount}
										</button>
										{author?.id === me.id && (
											<button
												onClick={() => remove(post.id)}
												className="text-xs text-neutral-400 hover:text-red-500"
											>
												Delete
											</button>
										)}
									</div>
								</CardContent>
							</Card>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

function ProfileSetup({
	existingHandles,
	onSaved,
}: {
	existingHandles: string[];
	onSaved: (p: Profile) => void;
}) {
	const [handle, setHandle] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [bio, setBio] = useState("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function save(e: React.FormEvent) {
		e.preventDefault();
		setError(null);
		const lower = handle.toLowerCase();
		if (existingHandles.includes(lower)) {
			setError(`@${lower} is taken`);
			return;
		}
		setSaving(true);
		try {
			const profile = (await callFn("upsertProfile", {
				handle: lower,
				displayName,
				bio,
			})) as Profile;
			onSaved(profile);
		} catch (e) {
			setError(String(e));
		} finally {
			setSaving(false);
		}
	}

	return (
		<Card>
			<CardHeader>
				<h2 className="text-sm font-medium">Set up your profile</h2>
			</CardHeader>
			<CardContent>
				<form onSubmit={save} className="space-y-3">
					<div>
						<label className="text-xs text-neutral-500 block mb-1">Handle</label>
						<Input
							value={handle}
							onChange={(e) => setHandle(e.target.value.toLowerCase())}
							placeholder="lowercase letters, digits, underscore"
							pattern="[a-z0-9_]{2,20}"
							required
						/>
					</div>
					<div>
						<label className="text-xs text-neutral-500 block mb-1">
							Display name
						</label>
						<Input
							value={displayName}
							onChange={(e) => setDisplayName(e.target.value)}
							required
						/>
					</div>
					<div>
						<label className="text-xs text-neutral-500 block mb-1">Bio</label>
						<Input
							value={bio}
							onChange={(e) => setBio(e.target.value)}
							placeholder="(optional)"
						/>
					</div>
					{error && <p className="text-xs text-red-500">{error}</p>}
					<Button type="submit" variant="primary" disabled={saving}>
						{saving ? "Saving…" : "Save"}
					</Button>
				</form>
			</CardContent>
		</Card>
	);
}
