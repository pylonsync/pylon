"use client";

import { useState } from "react";
import { callFn } from "@pylonsync/react";

// Footer "get updates" capture. Visitors are anonymous; the public
// joinUpdates mutation validates + dedupes server-side, so this stays a
// dumb form: submit, show one of three states, never block reading the
// page. Success sticks (no reset) — the point is the address, not a
// conversation.
export function UpdatesSignup() {
	const [email, setEmail] = useState("");
	const [state, setState] = useState<"idle" | "busy" | "done" | "error">(
		"idle",
	);

	async function submit(e: React.FormEvent) {
		e.preventDefault();
		if (state === "busy" || state === "done") return;
		const value = email.trim();
		if (!value) return;
		setState("busy");
		try {
			await callFn("joinUpdates", { email: value, source: "footer" });
			setState("done");
		} catch {
			setState("error");
		}
	}

	if (state === "done") {
		return (
			<p className="mt-5 text-[13px] leading-[1.5] text-[var(--color-ink-3)]">
				You're on the list — launch notes only, no noise.
			</p>
		);
	}

	return (
		<form onSubmit={submit} className="mt-5">
			<label
				htmlFor="footer-updates-email"
				className="font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]"
			>
				Get updates
			</label>
			<div className="mt-2 flex max-w-[300px] items-center gap-2">
				<input
					id="footer-updates-email"
					type="email"
					required
					value={email}
					onChange={(e) => {
						setEmail(e.target.value);
						if (state === "error") setState("idle");
					}}
					placeholder="you@company.com"
					className="h-9 min-w-0 flex-1 rounded-md border border-[var(--color-rule)] bg-transparent px-3 text-[13px] text-[var(--color-ink)] outline-none transition-colors placeholder:text-[var(--color-ink-4)] focus:border-[var(--color-ink-3)]"
				/>
				<button
					type="submit"
					disabled={state === "busy"}
					className="h-9 shrink-0 rounded-md bg-[var(--color-ink)] px-3.5 text-[13px] font-medium text-[var(--color-paper)] transition-opacity hover:opacity-85 disabled:opacity-60"
				>
					{state === "busy" ? "…" : "Subscribe"}
				</button>
			</div>
			{state === "error" ? (
				<p className="mt-2 text-[12px] text-red-600">
					That didn't go through — check the address and try again.
				</p>
			) : null}
		</form>
	);
}
