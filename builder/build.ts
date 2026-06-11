// pylon-builder — one-shot build step for the Pylon Cloud build-artifact
// pipeline (RFC pylon-cloud/docs/rfc-deploy-build-pipeline.md).
//
// Runs on a throwaway, isolated Fly machine: fetch the customer's source (via a
// signed url — NO GitHub token ever reaches the builder), `bun install`, run
// the app build, assemble a FULL-PREBUILT bundle (code + node_modules +
// web/dist + public/), upload it to Tigris, and write a COMPLETE marker the
// control plane polls for (the machine self-destructs, so its exit code is
// gone). Exit 0 = bundle + marker uploaded; non-zero = the control plane sees
// no marker and marks the deploy failed, leaving the live app untouched.
//
// Env contract (all signed urls minted by the control plane; the builder holds
// no standing credentials):
//   PYLON_BUILD_SOURCE_URL          signed GET → source.tar.gz
//   PYLON_ARTIFACT_PUT_URL          signed PUT ← bundle.tar.gz  (ct: application/gzip)
//   PYLON_ARTIFACT_COMPLETE_PUT_URL signed PUT ← .complete JSON {sha256,size}
//   PYLON_BUILD_ID                  for logs

import { rm } from "node:fs/promises";

const SRC = "/build/src";
const SOURCE_TAR = "/build/source.tar.gz";
const BUNDLE = "/build/bundle.tar.gz";

function need(name: string): string {
	const v = process.env[name];
	if (!v) {
		console.error(`[builder] missing required env ${name}`);
		process.exit(2);
	}
	return v;
}

async function sh(cmd: string[], cwd?: string): Promise<void> {
	console.log(`[builder] $ ${cmd.join(" ")}`);
	const proc = Bun.spawn(cmd, {
		cwd,
		stdout: "inherit",
		stderr: "inherit",
		env: process.env as Record<string, string>,
	});
	const code = await proc.exited;
	if (code !== 0) {
		throw new Error(`command failed (exit ${code}): ${cmd.join(" ")}`);
	}
}

async function sha256(path: string): Promise<{ hex: string; size: number }> {
	const file = Bun.file(path);
	const size = file.size;
	const hasher = new Bun.CryptoHasher("sha256");
	const stream = file.stream();
	for await (const chunk of stream) hasher.update(chunk);
	return { hex: hasher.digest("hex"), size };
}

async function main() {
	const sourceUrl = need("PYLON_BUILD_SOURCE_URL");
	const putUrl = need("PYLON_ARTIFACT_PUT_URL");
	const completeUrl = need("PYLON_ARTIFACT_COMPLETE_PUT_URL");
	const buildId = process.env.PYLON_BUILD_ID ?? "unknown";
	console.log(`[builder] build ${buildId} starting`);

	// 1. Fetch + extract source.
	await sh(["mkdir", "-p", SRC]);
	const srcResp = await fetch(sourceUrl);
	if (!srcResp.ok) throw new Error(`fetch source: HTTP ${srcResp.status}`);
	await Bun.write(SOURCE_TAR, srcResp);
	await sh(["tar", "xzf", SOURCE_TAR, "-C", SRC]);

	// 2. Install deps.
	//
	//    First neutralize `workspace:*` protocol specifiers. A deploy source is
	//    a single app carved out of a monorepo (an `examples/*` app, a customer
	//    monorepo package, …), so its `package.json` can still reference sibling
	//    workspace packages — most commonly `"@pylonsync/*": "workspace:*"`.
	//    `bun install` cannot resolve `workspace:` without the workspace root,
	//    so it ABORTS before installing the app's real npm deps (clsx,
	//    lucide-react, …). The app then boots with a half-populated
	//    node_modules and its SSR client bundle fails to resolve those modules.
	//    Rewrite each `workspace:` spec to the published SDK version (passed by
	//    the control plane as PYLON_SDK_VERSION, matching the runtime image;
	//    falls back to "latest") so they resolve from npm like everything else.
	const pkgPath = `${SRC}/package.json`;
	let rewroteWorkspace = false;
	try {
		const pkg = (await Bun.file(pkgPath).json()) as Record<string, unknown>;
		const sdkVersion = process.env.PYLON_SDK_VERSION || "latest";
		for (const field of [
			"dependencies",
			"devDependencies",
			"optionalDependencies",
			"peerDependencies",
		]) {
			const deps = pkg[field] as Record<string, string> | undefined;
			if (!deps) continue;
			for (const [name, spec] of Object.entries(deps)) {
				if (typeof spec === "string" && spec.startsWith("workspace:")) {
					deps[name] = sdkVersion;
					rewroteWorkspace = true;
				}
			}
		}
		if (rewroteWorkspace) {
			await Bun.write(pkgPath, JSON.stringify(pkg, null, 2));
			console.log(
				`[builder] rewrote workspace:* deps → ${sdkVersion} (standalone install)`,
			);
		}
	} catch (err) {
		console.error(`[builder] could not pre-process package.json: ${err}`);
	}

	//    Install (frozen — the lockfile is the contract; a drift is a real error
	//    the customer must resolve, not silently re-resolved). EXCEPT when we
	//    rewrote workspace specifiers: the lockfile that rode along is the
	//    monorepo's — it references sibling packages this single-app tree
	//    doesn't contain, so a frozen install against it always fails. Drop it
	//    so bun re-resolves cleanly from the rewritten package.json.
	if (rewroteWorkspace) {
		await rm(`${SRC}/bun.lock`, { force: true });
		await rm(`${SRC}/bun.lockb`, { force: true });
	}
	const hasLock =
		!rewroteWorkspace &&
		((await Bun.file(`${SRC}/bun.lock`).exists()) ||
			(await Bun.file(`${SRC}/bun.lockb`).exists()));
	await sh(
		hasLock ? ["bun", "install", "--frozen-lockfile"] : ["bun", "install"],
		SRC,
	);

	// 3. Run the app's build script if it declares one (frontend build,
	//    build:content hooks, etc.).
	const pkg = await Bun.file(`${SRC}/package.json`)
		.json()
		.catch(() => ({}) as Record<string, unknown>);
	const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
	if (scripts.build) {
		await sh(["bun", "run", "build"], SRC);
	} else {
		console.log("[builder] no build script — skipping");
	}

	// 4. Assemble the FULL-PREBUILT bundle: everything the runtime needs at
	//    boot (app.ts, functions/, lib/, node_modules/, web/dist/, public/, …),
	//    minus VCS + dev/build-cache debris. Deterministic-ish: sorted names.
	await sh([
		"tar",
		"czf",
		BUNDLE,
		"-C",
		SRC,
		"--exclude=./.git",
		"--exclude=./.github",
		"--exclude=./.pylon",
		"--exclude=./.cache",
		"--exclude=./node_modules/.cache",
		"--exclude=./.turbo",
		"--exclude=./source.tar.gz",
		".",
	]);

	// 5. Hash for integrity (the app machine verifies this before extracting).
	const { hex, size } = await sha256(BUNDLE);
	console.log(`[builder] bundle ${(size / 1048576).toFixed(1)} MB sha256=${hex}`);

	// 6. Upload the bundle (content-type must match the presigned signature).
	const putResp = await fetch(putUrl, {
		method: "PUT",
		headers: { "content-type": "application/gzip" },
		body: Bun.file(BUNDLE),
	});
	if (!putResp.ok) throw new Error(`upload bundle: HTTP ${putResp.status}`);

	// 7. Write the COMPLETE marker LAST — its presence + contents are how the
	//    control plane knows the build succeeded and learns the verified hash.
	const marker = JSON.stringify({
		sha256: hex,
		size,
		buildId,
		ok: true,
	});
	const completeResp = await fetch(completeUrl, {
		method: "PUT",
		headers: { "content-type": "application/json" },
		body: marker,
	});
	if (!completeResp.ok) {
		throw new Error(`write complete marker: HTTP ${completeResp.status}`);
	}

	console.log(`[builder] build ${buildId} done`);
}

main().catch((err) => {
	console.error(`[builder] FAILED: ${err?.message ?? err}`);
	process.exit(1);
});
