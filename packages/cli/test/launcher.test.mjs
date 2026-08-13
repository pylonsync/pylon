// The launcher picks the platform binary, and picking it wrong is a dead end
// for the user: the message names a machine they don't have and offers no way
// forward. That happened for real — an Apple Silicon Mac was told "Intel Macs
// aren't currently supported" because node itself was an x64 build running
// under Rosetta, which Migration Assistant produces on any new Mac set up from
// an Intel one.
//
// These tests drive the real bin/pylon.js in a subprocess with process.platform
// and process.arch redefined and a stub `sysctl` ahead of it on PATH, so the
// darwin branches are exercised on Linux CI too.
//
// The launcher under test is a COPY in a temp dir, deliberately: require.resolve
// runs relative to the launcher's own path, so a copy outside the workspace
// cannot see @pylonsync/cli-darwin-arm64 and always takes the not-installed
// branch. Run in place, the result would depend on whether a binary happens to
// be staged locally — which is exactly the kind of machine-dependent test that
// passes for one person and fails for everyone else.

import { test, expect, beforeAll, afterAll } from "bun:test";
import { spawnSync } from "node:child_process";
import {
	mkdtempSync,
	writeFileSync,
	copyFileSync,
	chmodSync,
	rmdirSync,
	unlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const REAL_LAUNCHER = join(
	dirname(fileURLToPath(import.meta.url)),
	"..",
	"bin",
	"pylon.js",
);

// Spawn node, not process.execPath — under `bun test` that would be bun, whose
// resolver finds the workspace package from the cwd and defeats the isolation
// below. Users run the launcher under node anyway (`#!/usr/bin/env node`), so
// this is also the faithful thing to exercise.
const NODE = "node";

let workDir;
let preload;
let isolatedLauncher;

beforeAll(() => {
	const probe = spawnSync(NODE, ["--version"], { encoding: "utf8" });
	if (probe.status !== 0) {
		throw new Error(
			"these tests need `node` on PATH — the launcher is a node script and " +
				"bun's module resolution would not reproduce what users hit",
		);
	}

	workDir = mkdtempSync(join(tmpdir(), "pylon-launcher-"));

	// Redefining the two properties the launcher reads. They're getters on the
	// real process object, so a plain assignment silently does nothing.
	preload = join(workDir, "fake-platform.cjs");
	writeFileSync(
		preload,
		`Object.defineProperty(process, "platform", { value: "darwin" });\n` +
			`Object.defineProperty(process, "arch", { value: "x64" });\n`,
	);

	isolatedLauncher = join(workDir, "pylon.js");
	copyFileSync(REAL_LAUNCHER, isolatedLauncher);
});

afterAll(() => {
	for (const f of [preload, isolatedLauncher, join(workDir, "sysctl")]) {
		try {
			unlinkSync(f);
		} catch {}
	}
	try {
		rmdirSync(workDir);
	} catch {}
});

/**
 * Install a stub `sysctl` answering `hw.optional.arm64` with `answer`, or
 * failing when `answer` is null — which is what a genuine Intel Mac does,
 * since the OID doesn't exist there.
 */
function stubSysctl(answer) {
	const stub = join(workDir, "sysctl");
	writeFileSync(
		stub,
		answer === null ? `#!/bin/sh\nexit 1\n` : `#!/bin/sh\necho "${answer}"\n`,
	);
	chmodSync(stub, 0o755);
}

/** Run the launcher as an x64-reporting process on darwin. */
function runAsRosetta({ answer, launcher = isolatedLauncher }) {
	stubSysctl(answer);
	return spawnSync(NODE, ["-r", preload, launcher, "--version"], {
		encoding: "utf8",
		env: { ...process.env, PATH: `${workDir}:${process.env.PATH}` },
	});
}

test("Apple Silicon under a Rosetta node resolves the arm64 binary", () => {
	const { stderr } = runAsRosetta({ answer: "1" });

	expect(stderr).toContain("cli-darwin-arm64");
	// The regression this guards: an M-series machine told it is an Intel one.
	expect(stderr).not.toContain("darwin-x64");
	expect(stderr).not.toContain("Intel Macs aren't currently supported");
});

test("the Rosetta failure explains the cause and how to override the cpu match", () => {
	const { stderr } = runAsRosetta({ answer: "1" });

	// This is the branch a Rosetta user actually lands on: the package manager
	// matched `cpu` against x64 at install time and skipped an arm64-only
	// optional dependency, so reinstalling the same way skips it again. The
	// message has to say that and give a command that works.
	expect(stderr).toContain("is not installed");
	expect(stderr).toContain("Rosetta");
	expect(stderr).toContain("--cpu=arm64");
	expect(stderr).toContain("arch -arm64 brew install node");
});

test("a genuine Intel Mac still gets the unsupported message", () => {
	const { stderr } = runAsRosetta({ answer: null });

	expect(stderr).toContain("no prebuilt binary for darwin-x64");
	expect(stderr).toContain("Intel Macs aren't currently supported");
});

test("hw.optional.arm64 answering 0 is treated as Intel, not Apple Silicon", () => {
	const { stderr } = runAsRosetta({ answer: "0" });

	expect(stderr).toContain("no prebuilt binary for darwin-x64");
});

test("a native process never probes sysctl", () => {
	// No preload, so this runs as the host's real platform/arch against a stub
	// sysctl that answers with nonsense. If the probe ran, the nonsense would
	// have to be ignored; skipping it entirely is what keeps every command on
	// the common path from paying for a subprocess.
	stubSysctl("definitely not a number");

	const { stderr } = spawnSync(
		NODE,
		[isolatedLauncher, "--version"],
		{
			encoding: "utf8",
			env: { ...process.env, PATH: `${workDir}:${process.env.PATH}` },
		},
	);

	expect(stderr).toContain(`${process.platform}-${process.arch}`);
});
