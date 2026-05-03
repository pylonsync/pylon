// Studio extensions registry.
//
// `studio.entry.tsx` (in the user's project) gets bundled to a single
// ESM file by the CLI and served at `/studio/extensions.js`. This
// module dynamic-imports that file at boot and stores its component
// maps so the rest of the Studio (column renderers, custom pages,
// custom layout slots) can resolve string ids to React components.
//
// Why globals + dynamic import: the bundle imports React from the
// browser-loaded host (via the import map injected at runtime) and
// exports its registry as `default`. We can't statically import it
// because Vite would try to bundle it into the main chunk. Dynamic
// import keeps the boundary clean.

import type { ComponentType } from "react";
import type { ColumnConfig } from "@/lib/studio-config";

export interface CellRendererProps {
	value: unknown;
	row: Record<string, unknown>;
	column: ColumnConfig;
}

export interface CustomPageProps {
	manifest: unknown;
	auth: unknown;
	api: <T = unknown>(path: string, opts?: unknown) => Promise<T>;
}

export interface ExtensionRegistry {
	renderers?: Record<string, ComponentType<CellRendererProps>>;
	pages?: Record<string, ComponentType<CustomPageProps>>;
	layouts?: {
		footer?: ComponentType;
		headerActions?: ComponentType;
	};
}

const REGISTRY: ExtensionRegistry = {
	renderers: {},
	pages: {},
	layouts: {},
};

let loadPromise: Promise<void> | null = null;

/**
 * Trigger the one-time dynamic import of `/studio/extensions.js`.
 * No-op when `config.hasExtensions` is false. Idempotent — repeated
 * calls reuse the same promise.
 */
export function loadExtensions(hasExtensions: boolean): Promise<void> {
	if (!hasExtensions) {
		return Promise.resolve();
	}
	if (loadPromise) return loadPromise;
	loadPromise = (async () => {
		try {
			const mod = (await import(
				/* @vite-ignore */ `${apiBase()}/studio/extensions.js`
			)) as { default?: ExtensionRegistry } & Partial<ExtensionRegistry>;
			const registry: ExtensionRegistry = mod.default ?? mod;
			Object.assign(REGISTRY.renderers!, registry.renderers ?? {});
			Object.assign(REGISTRY.pages!, registry.pages ?? {});
			Object.assign(REGISTRY.layouts!, registry.layouts ?? {});
		} catch (err) {
			// Failure shouldn't take down the Studio — extensions are
			// purely additive. Log so the operator sees what happened.
			console.error("[pylon-studio] failed to load extensions:", err);
		}
	})();
	return loadPromise;
}

function apiBase(): string {
	if (typeof window === "undefined") return "";
	return (window as { __PYLON_API__?: string }).__PYLON_API__ ?? "";
}

export function getExtensionRenderer(
	id: string,
): ComponentType<CellRendererProps> | undefined {
	return REGISTRY.renderers?.[id];
}

export function getExtensionPage(
	id: string,
): ComponentType<CustomPageProps> | undefined {
	return REGISTRY.pages?.[id];
}

export function getExtensionLayoutSlot(
	slot: "footer" | "headerActions",
): ComponentType | undefined {
	return REGISTRY.layouts?.[slot];
}
