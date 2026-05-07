"use client";

import { useState, useTransition } from "react";
import { Button, Input, Card, CardHeader, CardContent } from "@__APP_NAME_KEBAB__/ui";

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

export function Feed({
	initialFeed,
	initialMe,
}: {
	initialFeed: FeedItem[];
	initialMe: Profile | null;
}) {
	const [feed, setFeed] = useState(initialFeed);
	const [me, setMe] = useState(initialMe);
	const [body, setBody] = useState("");
	const [pending, startTransition] = useTransition();

	if (!me) {
		return <ProfileSetup onSaved={(p) => setMe(p)} />;
	}

	async function post() {
		const trimmed = body.trim();
		if (!trimmed) return;
		setBody("");
		startTransition(async () => {
			const res = await fetch("/api/fn/createPost", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ body: trimmed }),
			});
			if (res.ok) {
				const item = (await res.json()) as FeedItem;
				setFeed((prev) => [item, ...prev]);
			} else {
				setBody(trimmed);
			}
		});
	}

	async function toggleLike(item: FeedItem) {
		// Optimistic
		setFeed((prev) =>
			prev.map((p) =>
				p.id === item.id
					? {
							...p,
							likedByMe: !p.likedByMe,
							likeCount: p.likeCount + (p.likedByMe ? -1 : 1),
						}
					: p,
			),
		);
		startTransition(async () => {
			const res = await fetch("/api/fn/toggleLike", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ postId: item.id }),
			});
			if (!res.ok) {
				// Revert
				setFeed((prev) =>
					prev.map((p) =>
						p.id === item.id
							? {
									...p,
									likedByMe: item.likedByMe,
									likeCount: item.likeCount,
								}
							: p,
					),
				);
			}
		});
	}

	async function remove(item: FeedItem) {
		const snapshot = feed;
		setFeed((prev) => prev.filter((p) => p.id !== item.id));
		startTransition(async () => {
			const res = await fetch("/api/fn/deletePost", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ id: item.id }),
			});
			if (!res.ok) setFeed(snapshot);
		});
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
						<button
							className="text-xs text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
							onClick={() => setMe(null)}
						>
							Edit profile
						</button>
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
							disabled={pending}
							className="w-full rounded-md border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm placeholder:text-neutral-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 resize-none"
						/>
						<div className="flex items-center justify-between">
							<span className="text-xs text-neutral-400">
								{body.length}/1000
							</span>
							<Button
								type="submit"
								variant="primary"
								disabled={pending || !body.trim()}
								size="sm"
							>
								Post
							</Button>
						</div>
					</form>
				</CardContent>
			</Card>

			{feed.length === 0 ? (
				<p className="text-sm text-neutral-500 text-center py-8">
					No posts yet. Be the first.
				</p>
			) : (
				<ul className="space-y-3">
					{feed.map((item) => (
						<li key={item.id}>
							<Card>
								<CardContent className="space-y-3">
									<div className="flex items-baseline justify-between">
										<div className="text-sm">
											<span className="font-medium">
												{item.author?.displayName ?? "Unknown"}
											</span>{" "}
											<span className="text-neutral-400 font-mono text-xs">
												@{item.author?.handle ?? "?"}
											</span>
										</div>
										<span className="text-xs text-neutral-400">
											{new Date(item.createdAt).toLocaleString(undefined, {
												month: "short",
												day: "numeric",
												hour: "numeric",
												minute: "2-digit",
											})}
										</span>
									</div>
									<p className="text-sm whitespace-pre-wrap break-words">
										{item.body}
									</p>
									<div className="flex items-center gap-2">
										<button
											onClick={() => toggleLike(item)}
											className={`text-xs font-mono px-2 py-1 rounded border transition-colors ${
												item.likedByMe
													? "border-pink-300 dark:border-pink-700 text-pink-600 dark:text-pink-300 bg-pink-50 dark:bg-pink-950"
													: "border-neutral-200 dark:border-neutral-800 text-neutral-500 hover:bg-neutral-50 dark:hover:bg-neutral-900"
											}`}
										>
											{item.likedByMe ? "♥" : "♡"} {item.likeCount}
										</button>
										{item.author?.id === me.id && (
											<button
												onClick={() => remove(item)}
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

function ProfileSetup({ onSaved }: { onSaved: (p: Profile) => void }) {
	const [handle, setHandle] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [bio, setBio] = useState("");
	const [pending, startTransition] = useTransition();
	const [error, setError] = useState<string | null>(null);

	async function save(e: React.FormEvent) {
		e.preventDefault();
		setError(null);
		startTransition(async () => {
			const res = await fetch("/api/fn/upsertProfile", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ handle, displayName, bio }),
			});
			if (res.ok) {
				const profile = (await res.json()) as Profile;
				onSaved(profile);
			} else {
				const body = await res.json().catch(() => ({}));
				setError(body?.message ?? "save failed");
			}
		});
	}

	return (
		<Card>
			<CardHeader>
				<h2 className="text-sm font-medium">Set up your profile</h2>
			</CardHeader>
			<CardContent>
				<form onSubmit={save} className="space-y-3">
					<div>
						<label className="text-xs text-neutral-500 block mb-1">
							Handle
						</label>
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
						<label className="text-xs text-neutral-500 block mb-1">
							Bio
						</label>
						<Input
							value={bio}
							onChange={(e) => setBio(e.target.value)}
							placeholder="(optional)"
						/>
					</div>
					{error && <p className="text-xs text-red-500">{error}</p>}
					<Button type="submit" variant="primary" disabled={pending}>
						{pending ? "Saving…" : "Save"}
					</Button>
				</form>
			</CardContent>
		</Card>
	);
}
