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
//
// `darwin-x64` (Intel Mac) and `linux-arm64` (ARM Linux: Raspberry Pi,
// AWS Graviton, etc.) intentionally omitted — those binaries aren't
// built in CI today. Intel Mac dropped because macos-13 GitHub-hosted
// runners are no longer reliably available and cross-compiling from
// arm64 macOS is blocked on libxmlsec1. ARM Linux dropped because
// cross-compiling libxmlsec1 from ubuntu-latest is non-trivial.
// Affected users hit the `UNSUPPORTED_HINTS` message below.
const PLATFORM_PACKAGES = {
	"darwin-arm64": "@pylonsync/cli-darwin-arm64",
	"linux-x64": "@pylonsync/cli-linux-x64",
};

const UNSUPPORTED_HINTS = {
	"darwin-x64":
		"Intel Macs aren't currently supported by the npm-distributed CLI " +
		"(macos-13 GitHub runners are gone and cross-compile from arm64 is " +
		"blocked on libxmlsec1). Apple Silicon Macs are supported.",
	"linux-arm64":
		"ARM Linux isn't currently supported by the npm-distributed CLI " +
		"(libxmlsec1 cross-compile from ubuntu-latest is unsolved).",
};

// `process.arch` is the architecture node was BUILT for, not the machine's.
// An x64 node on an Apple Silicon Mac runs under Rosetta and reports "x64",
// which would send an M-series user to the "Intel Macs aren't supported"
// message above — a wrong answer with no way out of it. That setup is common
// rather than exotic: Migration Assistant carries the Intel node and the
// /usr/local Homebrew over to the new Mac, and both keep working, so nothing
// signals that the toolchain is now the odd one out.
//
// Ask the kernel about the hardware instead. `hw.optional.arm64` is 1 on
// every Apple Silicon Mac and readable from inside a translated process.
// Only darwin-x64 pays for the probe; a native arm64 or Linux node returns
// immediately without spawning anything.
function hardwareArch() {
	if (process.platform !== "darwin" || process.arch !== "x64") {
		return process.arch;
	}
	const probe = spawnSync("sysctl", ["-n", "hw.optional.arm64"], {
		encoding: "utf8",
	});
	return probe.status === 0 && probe.stdout.trim() === "1" ? "arm64" : "x64";
}

function resolveBinary() {
	// Escape hatch for a locally-built binary (esbuild's ESBUILD_BINARY_PATH
	// pattern). Without this, a `cargo install`ed or hand-built `pylon` is
	// invisible to `bun dev` / `npm run dev`, because bun puts node_modules/.bin
	// first on PATH and the script resolves THIS shim → the published prebuilt.
	// Point PYLON_BIN at your build to test a fix before it ships:
	//   PYLON_BIN=$HOME/.cargo/bin/pylon bun dev
	const override = process.env.PYLON_BIN;
	if (override) {
		if (!existsSync(override)) {
			throw new Error(
				`@pylonsync/cli: PYLON_BIN is set to "${override}" but no file ` +
					`exists there.`,
			);
		}
		return override;
	}

	const arch = hardwareArch();
	const translated = arch !== process.arch;
	const key = `${process.platform}-${arch}`;
	const pkg = PLATFORM_PACKAGES[key];
	if (!pkg) {
		const hint = UNSUPPORTED_HINTS[key]
			? `\n${UNSUPPORTED_HINTS[key]}`
			: "";
		throw new Error(
			`@pylonsync/cli: no prebuilt binary for ${key}.${hint}\n` +
				`Supported via npm: ${Object.keys(PLATFORM_PACKAGES).join(", ")}.\n` +
				`Build from source: https://github.com/pylonsync/pylon`,
		);
	}

	// require.resolve returns the package's main file; the binary lives
	// at <package-root>/bin/pylon. Walk up from main to find the dir.
	let entry;
	try {
		entry = require.resolve(`${pkg}/package.json`);
	} catch (e) {
		// Package managers match optionalDependencies against the `cpu` field
		// using the architecture of the running node, so a Rosetta node skips
		// the arm64 binary at install time. The package is genuinely absent
		// here and reinstalling the same way will skip it again — say so, and
		// give the flags that override the match.
		const cause = translated
			? `node is an x64 build running under Rosetta, so the install ` +
				`matched optional dependencies against x64 and skipped the ` +
				`arm64 binary. The machine itself is Apple Silicon.\n\n` +
				`Fix it for good by installing an arm64 node (the x64 one came ` +
				`over from an Intel Mac):\n` +
				`  arch -arm64 brew install node\n\n` +
				`Or install the binary for this machine directly:\n` +
				`  npm install --os=darwin --cpu=arm64 ${pkg}\n`
			: `This usually means npm/bun install skipped the optional ` +
				`dependency.\n` +
				`Try: npm install --no-optional=false @pylonsync/cli\n`;
		throw new Error(
			`@pylonsync/cli: ${pkg} is not installed.\n${cause}\n` +
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
