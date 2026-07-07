// ---------------------------------------------------------------------------
// Dynamic OpenGraph image rendering — the JSX → PNG pipeline.
//
// This is the server-only engine behind the `opengraph-image.tsx` file
// convention (Next.js `next/og` parity). A user module default-exports a
// function returning an `ImageResponse` (or a raw React element); the SSR
// runner calls it, hands the element here, and we:
//
//   1. Satori renders the React element tree to an SVG string. Satori is a
//      flexbox-only layout engine — multi-child nodes need explicit
//      `display: flex` (documented; same constraint as next/og).
//   2. resvg (WASM) rasterizes the SVG to a PNG.
//
// Both deps resolve from the FRAMEWORK's node_modules (this file lives under
// /pylon/packages/functions/src, so a bare import walks up to /pylon/
// node_modules) — deliberately NOT the user cwd (unlike react/react-dom).
// resvg is the WASM build (`@resvg/resvg-wasm`); the native napi build is
// unreliable under Bun. WASM under Bun is proven by loro-crdt in the same
// runtime.
//
// Fonts: Satori CANNOT synthesize text without a font buffer, and cannot
// parse woff2 or variable fonts. We bundle static Inter (400 + 600) as the
// default; apps override via `ImageResponse`'s `fonts` option.
// ---------------------------------------------------------------------------

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Satori's font descriptor.
export interface OgFont {
	name: string;
	data: ArrayBuffer | Buffer | Uint8Array;
	weight?: 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900;
	style?: "normal" | "italic";
}

export interface RenderOgOptions {
	width?: number;
	height?: number;
	/** Extra/override fonts. When omitted, the bundled Inter (400+600) is used. */
	fonts?: OgFont[];
	/**
	 * Emoji provider passthrough for Satori (`graphemeImages` / `loadAdditionalAsset`).
	 * Left undefined by default — emoji render as tofu unless the app supplies fonts.
	 */
	loadAdditionalAsset?: (
		code: string,
		text: string,
	) => Promise<string> | string;
}

const DEFAULT_WIDTH = 1200;
const DEFAULT_HEIGHT = 630;

// ---- lazy, memoized module + wasm init -----------------------------------
// satori + resvg are only pulled in when an OG render actually happens, and
// the wasm is initialized exactly once per Bun process (init is not
// re-entrant — a second initWasm throws).

let satoriMod: Promise<typeof import("satori")> | null = null;
function loadSatori() {
	if (!satoriMod) satoriMod = import("satori");
	return satoriMod;
}

let resvgReady: Promise<typeof import("@resvg/resvg-wasm")> | null = null;
function loadResvg() {
	if (!resvgReady) {
		resvgReady = (async () => {
			const mod = await import("@resvg/resvg-wasm");
			// Locate the wasm binary next to the package entry and init once.
			const entry = fileURLToPath(import.meta.resolve("@resvg/resvg-wasm"));
			const wasmPath = entry.replace(/index\.[^/]+$/, "index_bg.wasm");
			await mod.initWasm(readFileSync(wasmPath));
			return mod;
		})();
	}
	return resvgReady;
}

// ---- default fonts (bundled static Inter) --------------------------------

let defaultFonts: OgFont[] | null = null;
function loadDefaultFonts(): OgFont[] {
	if (defaultFonts) return defaultFonts;
	const read = (rel: string) =>
		readFileSync(fileURLToPath(new URL(rel, import.meta.url)));
	defaultFonts = [
		{
			name: "Inter",
			data: read("../assets/fonts/Inter-Regular.ttf"),
			weight: 400,
			style: "normal",
		},
		{
			name: "Inter",
			data: read("../assets/fonts/Inter-SemiBold.ttf"),
			weight: 600,
			style: "normal",
		},
	];
	return defaultFonts;
}

/**
 * Render a React element to a PNG buffer via Satori + resvg.
 *
 * `element` is whatever the user's `opengraph-image.tsx` produced — a React
 * element created by the *user's* react. Satori consumes it structurally
 * (by `type`/`props`), so the react instance doesn't matter.
 */
export async function renderOgImage(
	element: unknown,
	options: RenderOgOptions = {},
): Promise<Uint8Array> {
	const width = options.width ?? DEFAULT_WIDTH;
	const height = options.height ?? DEFAULT_HEIGHT;
	const fonts = options.fonts?.length ? options.fonts : loadDefaultFonts();

	const { default: satori } = await loadSatori();
	// Satori's types want ReactNode; the element is structurally compatible.
	const svg = await satori(element as never, {
		width,
		height,
		fonts: fonts as never,
		// Our public type is friendlier (sync-or-async → string); Satori's exact
		// signature is stricter but tolerant at runtime.
		loadAdditionalAsset: options.loadAdditionalAsset as never,
	});

	const { Resvg } = await loadResvg();
	const resvg = new Resvg(svg, {
		fitTo: { mode: "width", value: width },
		font: { loadSystemFonts: false },
	});
	return resvg.render().asPng();
}
