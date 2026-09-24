// Print the dependencies of local @pylonsync packages that an app does not
// have installed, as `name@range` lines for `bun add`.
//
// The smoke scripts install the published @pylonsync packages and copy the
// local sources over them. A dependency the local source added since the
// last release is missing from that install; this finds it.
//
// Usage: node tools/missing-local-deps.mjs <app-dir> <package-dir>...
import fs from "node:fs";
import path from "node:path";

const [appDir, ...pkgDirs] = process.argv.slice(2);
if (!appDir || pkgDirs.length === 0) {
	console.error("usage: missing-local-deps.mjs <app-dir> <package-dir>...");
	process.exit(2);
}

const missing = new Map();
for (const dir of pkgDirs) {
	const pkg = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
	for (const [name, range] of Object.entries(pkg.dependencies ?? {})) {
		if (name.startsWith("@pylonsync/") || range.startsWith("workspace:")) continue;
		if (fs.existsSync(path.join(appDir, "node_modules", name, "package.json"))) continue;
		missing.set(name, range);
	}
}
for (const [name, range] of missing) console.log(`${name}@${range}`);
