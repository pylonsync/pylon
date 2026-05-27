"use client";

import {
	type FormEvent,
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { db } from "@pylonsync/react";
import { cn } from "../lib/cn";

export interface ChatMessage {
	role: "user" | "assistant" | "system";
	content: string;
}

export interface ChatBotProps {
	/**
	 * Name of the server-side function that drives the model. It runs
	 * `ctx.llm.*` and emits its response as SSE chunks via
	 * `db.streamFn`. The handler receives `{ messages, ...extra }`.
	 */
	fn: string;
	/** Extra args merged into every call (e.g., `{model: "haiku"}`). */
	extraArgs?: Record<string, unknown>;
	/** System prompt prepended to every turn. */
	systemPrompt?: string;
	/** Initial messages to display (e.g., a welcome from the assistant). */
	initialMessages?: ChatMessage[];
	/** Placeholder shown in the input. */
	placeholder?: string;
	/** Header copy. */
	title?: ReactNode;
	className?: string;
}

/**
 * Drop-in chat UI for a server-side LLM handler. Wires the assistant
 * turn to `db.streamFn(fn, { messages })`; each yielded chunk is parsed
 * as a JSON event of the shape `{type: "text", delta: string}` (the
 * default emitted by the framework's streaming helpers). Plain string
 * chunks are also accepted so this works with apps that just yield
 * `event.delta` directly.
 *
 * Server-side authoring (5 lines):
 * ```ts
 * export default action({
 *   args: { messages: v.array(v.object({ role: v.string(), content: v.string() })) },
 *   async handler(ctx, { messages }) {
 *     return ctx.llm.streamMessage({ model: "claude-haiku", messages });
 *   },
 * });
 * ```
 */
export function ChatBot({
	fn,
	extraArgs,
	systemPrompt,
	initialMessages,
	placeholder = "Ask anything…",
	title = "Assistant",
	className,
}: ChatBotProps) {
	const [messages, setMessages] = useState<ChatMessage[]>(
		initialMessages ?? [],
	);
	const [draft, setDraft] = useState("");
	const [streaming, setStreaming] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const scrollerRef = useRef<HTMLDivElement | null>(null);

	// Keep the latest message visible without yanking scroll if the user
	// has manually scrolled up. Simple heuristic: only auto-scroll when
	// the user is already at the bottom.
	useEffect(() => {
		const el = scrollerRef.current;
		if (!el) return;
		const nearBottom =
			el.scrollHeight - el.scrollTop - el.clientHeight < 80;
		if (nearBottom) el.scrollTop = el.scrollHeight;
	}, [messages]);

	const send = useCallback(
		async (e?: FormEvent) => {
			e?.preventDefault();
			const text = draft.trim();
			if (!text || streaming) return;
			setError(null);

			const userTurn: ChatMessage = { role: "user", content: text };
			const turns: ChatMessage[] = systemPrompt
				? [{ role: "system", content: systemPrompt }, ...messages, userTurn]
				: [...messages, userTurn];

			setMessages((prev) => [
				...prev,
				userTurn,
				{ role: "assistant", content: "" },
			]);
			setDraft("");
			setStreaming(true);

			try {
				const args = { messages: turns, ...(extraArgs ?? {}) };
				let acc = "";
				for await (const chunk of db.streamFn(fn, args)) {
					const delta = extractDelta(chunk);
					if (!delta) continue;
					acc += delta;
					setMessages((prev) => {
						const next = prev.slice();
						const idx = next.length - 1;
						if (idx >= 0 && next[idx]?.role === "assistant") {
							next[idx] = { role: "assistant", content: acc };
						}
						return next;
					});
				}
			} catch (err) {
				setError(err instanceof Error ? err.message : "Stream failed.");
				setMessages((prev) => prev.slice(0, -1));
			} finally {
				setStreaming(false);
			}
		},
		[draft, extraArgs, fn, messages, streaming, systemPrompt],
	);

	function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			void send();
		}
	}

	return (
		<div
			className={cn(
				"mx-auto flex h-[480px] w-full max-w-xl flex-col overflow-hidden rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] shadow-sm",
				className,
			)}
		>
			<div className="border-b border-[var(--pylon-rule,#e5e7eb)] px-4 py-2.5">
				<p className="text-sm font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
					{title}
				</p>
			</div>
			<div
				ref={scrollerRef}
				className="flex-1 space-y-3 overflow-y-auto px-4 py-4"
			>
				{messages.length === 0 ? (
					<p className="text-center text-xs text-[var(--pylon-ink-3,#71717a)]">
						{placeholder}
					</p>
				) : null}
				{messages.map((m, i) => (
					<MessageBubble key={i} message={m} />
				))}
				{error ? (
					<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
						{error}
					</p>
				) : null}
			</div>
			<form
				onSubmit={send}
				className="flex items-end gap-2 border-t border-[var(--pylon-rule,#e5e7eb)] p-3"
			>
				<textarea
					value={draft}
					onChange={(e) => setDraft(e.target.value)}
					onKeyDown={onKeyDown}
					placeholder={placeholder}
					rows={1}
					className="flex-1 resize-none rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none"
				/>
				<button
					type="submit"
					disabled={streaming || !draft.trim()}
					className="rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
				>
					{streaming ? "…" : "Send"}
				</button>
			</form>
		</div>
	);
}

function MessageBubble({ message }: { message: ChatMessage }) {
	const isUser = message.role === "user";
	return (
		<div className={cn("flex", isUser ? "justify-end" : "justify-start")}>
			<div
				className={cn(
					"max-w-[80%] whitespace-pre-wrap rounded-2xl px-3 py-2 text-sm",
					isUser
						? "bg-[var(--pylon-ink,#0a0a0a)] text-[var(--pylon-paper,#ffffff)]"
						: "bg-[var(--pylon-paper-2,#f4f4f5)] text-[var(--pylon-ink,#0a0a0a)]",
				)}
			>
				{message.content || (
					<span className="text-[var(--pylon-ink-3,#71717a)]">…</span>
				)}
			</div>
		</div>
	);
}

/**
 * Normalise streamed chunks. The framework's streaming helpers emit
 * `{type: "text", delta: string}` JSON; apps writing raw streams may
 * just yield strings. Anything else is ignored — defensive parsing
 * keeps the UI from blowing up on tool-call events the chatbot doesn't
 * render yet.
 */
function extractDelta(chunk: unknown): string {
	if (typeof chunk === "string") return chunk;
	if (chunk && typeof chunk === "object") {
		const obj = chunk as { type?: string; delta?: unknown; text?: unknown };
		if (typeof obj.delta === "string") return obj.delta;
		if (obj.type === "text" && typeof obj.text === "string") return obj.text;
	}
	return "";
}
