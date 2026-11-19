// Row counts for the sidebar.
//
// A count comes from the list endpoint: a request for one row returns the
// total. The counts load a few at a time after the first paint, and a list
// page writes the total it already has.

import { useSyncExternalStore } from "react";
import { api } from "@/lib/pylon";

/** Entities past this number get no count, to keep the load small. */
const MAX_ENTITIES = 60;
const CONCURRENCY = 4;

let counts: Readonly<Record<string, number>> = {};
const listeners = new Set<() => void>();

function emit() {
	for (const l of listeners) l();
}

function subscribe(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function setCount(entity: string, total: number): void {
	if (counts[entity] === total) return;
	counts = { ...counts, [entity]: total };
	emit();
}

export async function loadCount(entity: string): Promise<void> {
	try {
		const res = await api<{ total?: number | null }>(
			`/api/entities/${encodeURIComponent(entity)}?page=1&per_page=1`,
		);
		if (typeof res?.total === "number") setCount(entity, res.total);
	} catch {
		// No count for this entity. The sidebar shows the name alone.
	}
}

export async function loadCounts(entities: string[]): Promise<void> {
	const queue = entities.slice(0, MAX_ENTITIES);
	const workers = Array.from({ length: CONCURRENCY }, async () => {
		for (let next = queue.shift(); next; next = queue.shift()) {
			await loadCount(next);
		}
	});
	await Promise.all(workers);
}

export function useCounts(): Readonly<Record<string, number>> {
	return useSyncExternalStore(subscribe, () => counts);
}
