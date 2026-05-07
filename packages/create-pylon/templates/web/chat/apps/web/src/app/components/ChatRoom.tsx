"use client";

import { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { Button, Input } from "@__APP_NAME_KEBAB__/ui";

type Room = {
	id: string;
	slug: string;
	name: string;
	createdAt: string;
};

type Message = {
	id: string;
	roomId: string;
	authorId: string;
	authorName: string;
	body: string;
	createdAt: string;
};

const NAME_KEY = "__APP_NAME_SNAKE___author_name";

export function ChatRoom() {
	// Live subscriptions. The sync engine pushes diffs over WebSocket
	// — every Room insert + every Message append from any other client
	// re-renders these components without a fetch / poll / refresh.
	const { data: rooms = [] } = db.useQuery<Room>("Room", {
		orderBy: { createdAt: "asc" },
	});
	const [activeId, setActiveId] = useState<string | null>(null);
	useEffect(() => {
		if (!activeId && rooms.length > 0) setActiveId(rooms[0].id);
	}, [rooms, activeId]);

	const { data: messages = [] } = db.useQuery<Message>("Message", {
		where: activeId ? { roomId: activeId } : undefined,
		orderBy: { createdAt: "asc" },
		limit: 200,
	});

	const [body, setBody] = useState("");
	const [authorName, setAuthorName] = useState("anonymous");
	const [sending, setSending] = useState(false);
	const scrollerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const stored =
			typeof window !== "undefined"
				? window.localStorage.getItem(NAME_KEY)
				: null;
		if (stored) setAuthorName(stored);
	}, []);

	useEffect(() => {
		// Auto-scroll to bottom when message count changes.
		scrollerRef.current?.scrollTo({
			top: scrollerRef.current.scrollHeight,
			behavior: "smooth",
		});
	}, [messages.length, activeId]);

	function persistName(next: string) {
		setAuthorName(next);
		window.localStorage.setItem(NAME_KEY, next);
	}

	const active = rooms.find((r) => r.id === activeId) ?? null;

	async function send() {
		const trimmed = body.trim();
		if (!trimmed || !active) return;
		setBody("");
		setSending(true);
		try {
			// Server validates body length / auth, runs ctx.db.insert,
			// the resulting change_event hits every subscriber's local
			// store. We don't append to local state ourselves — the
			// useQuery above re-renders when the new row lands.
			await callFn("sendMessage", {
				roomId: active.id,
				body: trimmed,
				authorName,
			});
		} finally {
			setSending(false);
		}
	}

	async function createRoom() {
		const name = window.prompt("Room name?");
		if (!name) return;
		const slug = name
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-|-$/g, "");
		try {
			const room = (await callFn("createRoom", { slug, name })) as Room;
			setActiveId(room.id);
		} catch (e) {
			window.alert(`Create failed: ${String(e)}`);
		}
	}

	return (
		<>
			<aside className="w-64 border-r border-neutral-200 dark:border-neutral-800 flex flex-col">
				<div className="p-4 border-b border-neutral-200 dark:border-neutral-800">
					<div className="flex items-center justify-between mb-2">
						<h2 className="text-sm font-medium">Rooms</h2>
						<button
							onClick={createRoom}
							className="text-xs text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
						>
							+ New
						</button>
					</div>
					<Input
						value={authorName}
						onChange={(e) => persistName(e.target.value)}
						placeholder="Your name"
						className="text-xs"
					/>
				</div>
				<ul className="flex-1 overflow-auto divide-y divide-neutral-200 dark:divide-neutral-800">
					{rooms.map((r) => (
						<li key={r.id}>
							<button
								onClick={() => setActiveId(r.id)}
								className={`w-full text-left px-4 py-2.5 text-sm hover:bg-neutral-50 dark:hover:bg-neutral-900 ${
									r.id === activeId
										? "bg-neutral-100 dark:bg-neutral-800 font-medium"
										: ""
								}`}
							>
								<div>{r.name}</div>
								<div className="text-xs text-neutral-400 font-mono">
									#{r.slug}
								</div>
							</button>
						</li>
					))}
					{rooms.length === 0 && (
						<li className="px-4 py-3 text-xs text-neutral-500">
							No rooms yet. Create one above.
						</li>
					)}
				</ul>
			</aside>

			<section className="flex-1 flex flex-col min-w-0">
				{active ? (
					<>
						<header className="px-6 py-3 border-b border-neutral-200 dark:border-neutral-800">
							<h1 className="text-sm font-medium">{active.name}</h1>
							<p className="text-xs text-neutral-400 font-mono">
								#{active.slug}
							</p>
						</header>

						<div
							ref={scrollerRef}
							className="flex-1 overflow-auto px-6 py-4 space-y-3"
						>
							{messages.length === 0 ? (
								<p className="text-sm text-neutral-500 text-center py-12">
									No messages yet. Say hi.
								</p>
							) : (
								messages.map((m) => (
									<div key={m.id} className="space-y-0.5">
										<div className="flex items-baseline gap-2">
											<span className="text-sm font-medium">
												{m.authorName}
											</span>
											<span className="text-xs text-neutral-400">
												{new Date(m.createdAt).toLocaleTimeString(undefined, {
													hour: "numeric",
													minute: "2-digit",
												})}
											</span>
										</div>
										<p className="text-sm whitespace-pre-wrap break-words">
											{m.body}
										</p>
									</div>
								))
							)}
						</div>

						<form
							onSubmit={(e) => {
								e.preventDefault();
								send();
							}}
							className="px-6 py-3 border-t border-neutral-200 dark:border-neutral-800 flex gap-2"
						>
							<Input
								value={body}
								onChange={(e) => setBody(e.target.value)}
								placeholder={`Message ${active.name}…`}
								disabled={sending}
								className="flex-1"
							/>
							<Button
								type="submit"
								variant="primary"
								disabled={sending || !body.trim()}
							>
								Send
							</Button>
						</form>
					</>
				) : (
					<div className="flex-1 flex items-center justify-center">
						<div className="text-center space-y-2">
							<p className="text-sm text-neutral-500">No room selected.</p>
							<button
								onClick={createRoom}
								className="text-sm text-blue-600 hover:underline"
							>
								Create the first one
							</button>
						</div>
					</div>
				)}
			</section>
		</>
	);
}
