"use client";

import { useEffect, useRef, useState, useTransition } from "react";
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

export function ChatRoom({
	initialRooms,
	initialActiveRoom,
	initialMessages,
}: {
	initialRooms: Room[];
	initialActiveRoom: Room | null;
	initialMessages: Message[];
}) {
	const [rooms, setRooms] = useState(initialRooms);
	const [active, setActive] = useState(initialActiveRoom);
	const [messages, setMessages] = useState(initialMessages);
	const [body, setBody] = useState("");
	const [authorName, setAuthorName] = useState("anonymous");
	const [pending, startTransition] = useTransition();
	const [pollIdx, setPollIdx] = useState(0);
	const scrollerRef = useRef<HTMLDivElement>(null);

	// Poll the active room every 1.5s. The framework supports a
	// WebSocket subscription path (db.useQuery) — we use polling here
	// so the scaffold has zero front-end SDK setup. Swap in
	// db.useQuery("Message", { roomId }) when you wire up
	// `@pylonsync/react`'s init() in your layout.
	useEffect(() => {
		if (!active) return;
		const t = setInterval(() => setPollIdx((n) => n + 1), 1500);
		return () => clearInterval(t);
	}, [active]);

	useEffect(() => {
		if (!active) return;
		void (async () => {
			const res = await fetch("/api/fn/roomMessages", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ roomId: active.id }),
			});
			if (res.ok) setMessages(await res.json());
		})();
	}, [active, pollIdx]);

	useEffect(() => {
		const stored =
			typeof window !== "undefined"
				? window.localStorage.getItem(NAME_KEY)
				: null;
		if (stored) setAuthorName(stored);
	}, []);

	useEffect(() => {
		// Auto-scroll to bottom on new messages.
		scrollerRef.current?.scrollTo({
			top: scrollerRef.current.scrollHeight,
			behavior: "smooth",
		});
	}, [messages]);

	function persistName(next: string) {
		setAuthorName(next);
		window.localStorage.setItem(NAME_KEY, next);
	}

	async function send() {
		const trimmed = body.trim();
		if (!trimmed || !active) return;
		setBody("");
		startTransition(async () => {
			const res = await fetch("/api/fn/sendMessage", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					roomId: active.id,
					body: trimmed,
					authorName,
				}),
			});
			if (res.ok) {
				const msg = (await res.json()) as Message;
				setMessages((prev) => [...prev, msg]);
			}
		});
	}

	async function createRoom() {
		const name = window.prompt("Room name?");
		if (!name) return;
		const slug = name
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-|-$/g, "");
		startTransition(async () => {
			const res = await fetch("/api/fn/createRoom", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ slug, name }),
			});
			if (res.ok) {
				const room = (await res.json()) as Room;
				setRooms((prev) => [...prev, room]);
				setActive(room);
			}
		});
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
								onClick={() => setActive(r)}
								className={`w-full text-left px-4 py-2.5 text-sm hover:bg-neutral-50 dark:hover:bg-neutral-900 ${
									r.id === active?.id
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

						<div ref={scrollerRef} className="flex-1 overflow-auto px-6 py-4 space-y-3">
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
								disabled={pending}
								className="flex-1"
							/>
							<Button
								type="submit"
								variant="primary"
								disabled={pending || !body.trim()}
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
