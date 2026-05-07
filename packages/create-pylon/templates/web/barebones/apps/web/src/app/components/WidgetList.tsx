"use client";

import { useState, useTransition } from "react";
import { Button, Input } from "@__APP_NAME_KEBAB__/ui";

type Widget = {
	id: string;
	name: string;
	count: number;
	createdAt: string;
};

export function WidgetList({ initialWidgets }: { initialWidgets: Widget[] }) {
	const [widgets, setWidgets] = useState(initialWidgets);
	const [name, setName] = useState("");
	const [pending, startTransition] = useTransition();

	async function add() {
		const trimmed = name.trim();
		if (!trimmed) return;
		setName("");
		startTransition(async () => {
			const res = await fetch("/api/fn/createWidget", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ name: trimmed }),
			});
			if (res.ok) {
				const widget = (await res.json()) as Widget;
				setWidgets((prev) => [widget, ...prev]);
			}
		});
	}

	return (
		<div className="space-y-4">
			<form
				onSubmit={(e) => {
					e.preventDefault();
					add();
				}}
				className="flex gap-2"
			>
				<Input
					value={name}
					onChange={(e) => setName(e.target.value)}
					placeholder="Name a widget…"
					disabled={pending}
					className="flex-1"
				/>
				<Button
					type="submit"
					variant="primary"
					disabled={pending || !name.trim()}
				>
					Create
				</Button>
			</form>

			{widgets.length === 0 ? (
				<p className="text-sm text-neutral-500 dark:text-neutral-400 text-center py-8">
					No widgets yet. Create one above.
				</p>
			) : (
				<ul className="divide-y divide-neutral-200 dark:divide-neutral-800 rounded-md border border-neutral-200 dark:border-neutral-800">
					{widgets.map((w) => (
						<li
							key={w.id}
							className="flex items-center justify-between gap-3 px-4 py-3 text-sm bg-white dark:bg-neutral-950"
						>
							<span className="font-medium">{w.name}</span>
							<span className="font-mono text-xs text-neutral-400">
								count: {w.count}
							</span>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}
