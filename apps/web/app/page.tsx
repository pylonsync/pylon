import { readFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { join } from "node:path";
import { LandingPage } from "./landing-page";

// pylonsync.com homepage. Sourced from a Claude Design handoff — the
// implementation lives in landing-page.tsx so the route file stays a
// thin entry-point and the design ships as a single client component
// (it owns the live-data simulation in the hero product mock).
//
// Server-side at build time: pull the framework version + the commit
// date of its tag so the hero can render "live last shipped" social
// proof without a runtime fetch. Both signals are pinned at build, so
// they update on every Vercel rebuild without any cron.
function getReleaseInfo(): { version: string; lastShippedISO: string | null } {
	let version = "0.3.149";
	let lastShippedISO: string | null = null;
	try {
		const sdkPkg = JSON.parse(
			readFileSync(join(process.cwd(), "..", "..", "packages", "sdk", "package.json"), "utf8"),
		);
		if (typeof sdkPkg.version === "string") version = sdkPkg.version;
	} catch {
		// Fall through with the safe default. The version is also bumped
		// at build by release.sh, so a missing package.json (e.g. in a
		// vendored prebuild) just hands the user a slightly stale number.
	}
	try {
		// Date of the matching git tag, formatted as ISO date. Falls back
		// to HEAD's commit date if the tag isn't present (e.g. local
		// dev). Vercel preserves .git so the lookup works in CI.
		const cmd = `git log -1 --format=%cs v${version} 2>/dev/null || git log -1 --format=%cs HEAD`;
		const out = execSync(cmd, { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
		if (/^\d{4}-\d{2}-\d{2}$/.test(out)) lastShippedISO = out;
	} catch {
		// No git available (vendored build) — drop the date silently.
	}
	return { version, lastShippedISO };
}

export default function Home() {
	const { version, lastShippedISO } = getReleaseInfo();
	return <LandingPage version={version} lastShippedISO={lastShippedISO} />;
}
