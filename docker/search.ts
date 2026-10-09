#!/usr/bin/env bun
// `search` — web search for the coding agent in a cloud dev workspace.
//
// Usage: search "query" [--num N]
//
// Posts to pylon-model-proxy's /v1/search with the box's token, so the box
// never holds a search provider key. Installed in the image as
// /usr/local/bin/search; dev-env-boot.sh tells the agent it exists.

const USAGE = 'Usage: search "query" [--num N]   (N is 1 to 10, default 5)';
const TEXT_LIMIT = 600;

type Result = {
	title?: string;
	url?: string;
	publishedDate?: string;
	text?: string;
};

function fail(message: string): never {
	console.error(message);
	process.exit(1);
}

function parseArgs(argv: string[]): { query: string; num: number } {
	const words: string[] = [];
	let num = 5;
	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "-h" || arg === "--help") {
			console.log(USAGE);
			process.exit(0);
		}
		if (arg === "--num" || arg === "-n" || arg.startsWith("--num=")) {
			const raw = arg.startsWith("--num=") ? arg.slice(6) : argv[++i];
			const n = Number(raw);
			if (!Number.isInteger(n) || n < 1 || n > 10) {
				fail(`search: --num must be a whole number from 1 to 10\n${USAGE}`);
			}
			num = n;
			continue;
		}
		words.push(arg);
	}
	const query = words.join(" ").trim();
	if (!query) fail(USAGE);
	return { query, num };
}

function clip(text: string): string {
	const flat = text.replace(/\s+/g, " ").trim();
	return flat.length > TEXT_LIMIT ? `${flat.slice(0, TEXT_LIMIT)}...` : flat;
}

async function main() {
	const { query, num } = parseArgs(process.argv.slice(2));

	const base = process.env.PYLON_DEV_MODEL_PROXY_URL?.replace(/\/+$/, "");
	const token = process.env.PYLON_DEV_MODEL_PROXY_TOKEN;
	if (!base || !token) {
		fail(
			"search: web search is not available in this workspace. PYLON_DEV_MODEL_PROXY_URL and PYLON_DEV_MODEL_PROXY_TOKEN must be set.",
		);
	}

	let res: Response;
	try {
		res = await fetch(`${base}/v1/search`, {
			method: "POST",
			headers: {
				authorization: `Bearer ${token}`,
				"content-type": "application/json",
			},
			body: JSON.stringify({ query, numResults: num }),
			signal: AbortSignal.timeout(30_000),
		});
	} catch (err) {
		fail(`search: request failed: ${err instanceof Error ? err.message : String(err)}`);
	}

	const body = await res.text();
	let data: { results?: Result[]; error?: { message?: string } } | undefined;
	try {
		data = JSON.parse(body);
	} catch {
		data = undefined;
	}

	if (!res.ok || !data || data.error) {
		const message =
			data?.error?.message ?? (body.trim().slice(0, 300) || res.statusText);
		fail(`search: ${message} (HTTP ${res.status})`);
	}

	const results = Array.isArray(data.results) ? data.results : [];
	if (results.length === 0) {
		console.log(`No results for "${query}".`);
		return;
	}

	const blocks = results.map((r, i) => {
		const lines = [`${i + 1}. ${r.title?.trim() || "(no title)"}`, `   ${r.url ?? ""}`];
		if (r.publishedDate) lines.push(`   Published: ${r.publishedDate}`);
		if (r.text) lines.push(`   ${clip(r.text)}`);
		return lines.join("\n");
	});
	console.log(blocks.join("\n\n"));
}

await main();
