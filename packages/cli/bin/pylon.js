#!/usr/bin/env node
// Pylon CLI dispatcher.
//
// Resolves the platform-specific binary package at runtime and execs it
// with the same argv. Mirrors the pattern used by esbuild, swc, turbo,
// rollup, etc.: one main package + N optionalDependencies, each shipping
// one prebuilt binary. No postinstall script — npm/bun install only the
// optional dep matching the current platform, and we look it up via
// require.resolve.
//
// Why not a single package with all binaries: tarball size. Each
// platform binary is ~16MB; bundling all four would balloon every
// install to 64MB regardless of platform. The optionalDependencies
// pattern keeps installs at ~16MB.

import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { existsSync } from "node:fs";

const require = createRequire(import.meta.url);

// Map (process.platform, process.arch) → package name. The name is also
// the platform-package's directory name on disk.
const PLATFORM_PACKAGES = {
	"darwin-arm64": "@pylonsync/cli-darwin-arm64",
	"darwin-x64": "@pylonsync/cli-darwin-x64",
	"linux-arm64": "@pylonsync/cli-linux-arm64",
	"linux-x64": "@pylonsync/cli-linux-x64",
};

function resolveBinary() {
	const key = `${process.platform}-${process.arch}`;
	const pkg = PLATFORM_PACKAGES[key];
	if (!pkg) {
		throw new Error(
			`@pylonsync/cli: no prebuilt binary for ${key}.\n` +
				`Supported: ${Object.keys(PLATFORM_PACKAGES).join(", ")}.\n` +
				`Build from source: https://github.com/pylonsync/pylon`,
		);
	}

	// require.resolve returns the package's main file; the binary lives
	// at <package-root>/bin/pylon. Walk up from main to find the dir.
	let entry;
	try {
		entry = require.resolve(`${pkg}/package.json`);
	} catch (e) {
		throw new Error(
			`@pylonsync/cli: ${pkg} is not installed.\n` +
				`This usually means npm/bun install skipped the optional dependency.\n` +
				`Try: npm install --no-optional=false @pylonsync/cli\n\n` +
				`Underlying error: ${e.message}`,
		);
	}

	const binPath = join(dirname(entry), "bin", "pylon");
	if (!existsSync(binPath)) {
		throw new Error(
			`@pylonsync/cli: binary missing at ${binPath}.\n` +
				`The platform package was installed but doesn't contain the binary.\n` +
				`Try reinstalling: npm install --force @pylonsync/cli`,
		);
	}
	return binPath;
}

let binary;
try {
	binary = resolveBinary();
} catch (err) {
	process.stderr.write(`${err.message}\n`);
	process.exit(1);
}

// Pass argv straight through. `inherit` for stdio so colors / progress
// bars / TTY-aware output work unchanged.
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

// Forward the binary's exit code so callers get an honest signal —
// `pylon dev && next dev` should fail fast if pylon dev crashes.
if (result.error) {
	process.stderr.write(`@pylonsync/cli: failed to spawn binary: ${result.error.message}\n`);
	process.exit(1);
}
process.exit(result.status ?? 0);
