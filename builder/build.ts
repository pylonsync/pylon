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

import { existsSync } from "node:fs";
import { readFile, readdir, rm } from "node:fs/promises";

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

// Like `sh` but returns success instead of throwing — for steps that are
// allowed to fail without failing the deploy (e.g. probing whether a
// frontend's deps are installable in this bundle).
async function trySh(cmd: string[], cwd?: string): Promise<boolean> {
	console.log(`[builder] $ ${cmd.join(" ")}`);
	const proc = Bun.spawn(cmd, {
		cwd,
		stdout: "inherit",
		stderr: "inherit",
		env: process.env as Record<string, string>,
	});
	return (await proc.exited) === 0;
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
	//    PYLON_APP_SUBDIR (set by the control plane) marks a MONOREPO deploy: the
	//    source is the whole (pruned) workspace and the app lives in a subdir.
	//    Two very different install strategies:
	const appSubdir = (process.env.PYLON_APP_SUBDIR ?? "").replace(
		/^\/+|\/+$/g,
		"",
	);
	const isWorkspaceDeploy = appSubdir.length > 0;
	// Directory the app's own build script (if any) runs in.
	const appDir = isWorkspaceDeploy ? `${SRC}/${appSubdir}` : SRC;

	if (isWorkspaceDeploy) {
		// Workspace deploy: install ONCE at the workspace root. bun resolves every
		// `workspace:*` specifier against the bundled sibling packages (no npm
		// round-trip, no version skew — this is exactly how it works in local dev)
		// and hoists them into the root node_modules. The app, built/booted from
		// its subdir, resolves them by walking up. The lockfile that rode along
		// matches the (rewritten) pruned workspace, but we don't force --frozen:
		// the pruning may have trimmed members, so let bun re-resolve cleanly.
		console.log(`[builder] monorepo workspace deploy — app subdir: ${appSubdir}`);
		await sh(["bun", "install"], SRC);
	} else {
		// Single-app deploy: the source is one app carved out of a monorepo, so
		// its package.json can still reference sibling workspace packages —
		// `"@pylonsync/*": "workspace:*"`. With no workspace root, `bun install`
		// can't resolve `workspace:` and ABORTS before installing the app's real
		// npm deps (clsx, lucide-react, …), leaving a half-populated node_modules.
		// Rewrite each `workspace:` spec to the published SDK version (PYLON_SDK_-
		// VERSION, matching the runtime image; falls back to "latest") so they
		// resolve from npm. NOTE: this only works for PUBLISHED workspace deps;
		// an unpublished one (e.g. examples/_shared) needs the monorepo path above.
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

		// Install (frozen — the lockfile is the contract). EXCEPT when we rewrote
		// workspace specifiers: the lockfile that rode along is the monorepo's, so
		// a frozen install against it always fails. Drop it so bun re-resolves.
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
	}

	// 3. Run the app's build script if it declares one (frontend build,
	//    build:content hooks, etc.). For a workspace deploy this is the app's
	//    OWN package.json in its subdir, run from there.
	const pkg = await Bun.file(`${appDir}/package.json`)
		.json()
		.catch(() => ({}) as Record<string, unknown>);
	const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
	if (scripts.build) {
		await sh(["bun", "run", "build"], appDir);
	} else {
		console.log("[builder] no build script — skipping");
	}

	// 3b. Pre-build the SPA frontend (Pylon's `web/` / `apps/web/` convention).
	//     The runtime builds this at boot otherwise — but that happens on the
	//     LIVE machine AFTER the deploy flips, so a frontend build failure ships
	//     a broken app. Building it HERE makes the deploy atomic: a failure
	//     throws → no COMPLETE marker → the control plane marks the deploy failed
	//     and never touches the live machine. The built dist/ rides along in the
	//     bundle (step 4) and frontend.rs serves `<app>/web/dist` first, so the
	//     runtime skips its cold build entirely (bun.rs ensure_frontend_built).
	//
	//     Only for WORKSPACE deploys: there `web/` is a real workspace member
	//     (root install resolves its `workspace:*` / `@pylonsync/*` deps against
	//     the bundled siblings, exactly like dev). A single-app deploy has no
	//     workspace root, so `web/`'s workspace specifiers wouldn't resolve here
	//     — leave those to the runtime, which stages the symlinks itself.
	if (isWorkspaceDeploy) {
		for (const rel of ["web", "apps/web"]) {
			const webDir = `${appDir}/${rel}`;
			const webPkgFile = Bun.file(`${webDir}/package.json`);
			if (!(await webPkgFile.exists())) continue;
			const webPkg = (await webPkgFile
				.json()
				.catch(() => ({}))) as { scripts?: Record<string, string> };
			if (!webPkg.scripts?.build) continue;
			if (await Bun.file(`${webDir}/dist/index.html`).exists()) {
				console.log(
					`[builder] ${rel}/dist already built — skipping frontend build`,
				);
				break;
			}
			// Ensure web/'s own deps are present (its build tool + npm deps).
			// The root install covers it when web/ is a real workspace member
			// (git deploys of the full monorepo); a CLI-pruned bundle may not
			// install web/, so install it here. If that can't resolve (e.g. a
			// pruned bundle where web/'s `workspace:*` deps aren't satisfiable),
			// DON'T fail the deploy — leave the frontend to the runtime, which
			// stages workspace symlinks the builder can't. A real build failure
			// (deps present, build broken) DOES fail the deploy, atomically.
			if (!(await trySh(["bun", "install"], webDir))) {
				console.log(
					`[builder] ${rel}: deps not installable here (pruned bundle?) — leaving frontend build to the runtime`,
				);
				break;
			}
			console.log(`[builder] building frontend: ${rel}`);
			await sh(["bun", "run", "build"], webDir);
			break;
		}
	}

	// 3c. Pre-build the SSR client bundle (.pylon/client-build/) ONCE here, and —
	//     when a CDN is configured — publish its content-hashed assets so they're
	//     served off the app machines entirely (CDN), not rebuilt lazily on every
	//     machine at boot. Building it once means every machine serves IDENTICAL
	//     hashed asset names (no cross-machine 404 during a rollout), and lets the
	//     manifest bake absolute CDN urls (PYLON_PUBLIC_PREFIX). The runtime
	//     reuses this prebuilt bundle via the `.prebuilt` marker.
	//
	//     STRICTLY OPT-IN + ATOMIC: only when PYLON_PUBLIC_PREFIX + the assets
	//     upload url are set (CDN mode) AND every step succeeds do we ship the
	//     prebuilt client-build + report `clientAssets:true`. Any miss → we remove
	//     the partial build, report `clientAssets:false`, and the runtime falls
	//     back to its own lazy per-machine build (today's behavior). A broken or
	//     unconfigured pre-build never ships a broken app.
	let clientAssetsPublished = false;
	const publicPrefix = process.env.PYLON_PUBLIC_PREFIX ?? "";
	const assetsPutUrl = process.env.PYLON_ASSETS_PUT_URL ?? "";
	const clientBuildDir = `${appDir}/.pylon/client-build`;
	if (publicPrefix && assetsPutUrl && existsSync(`${appDir}/app`)) {
		try {
			console.log(`[builder] pre-building SSR client bundle (public_prefix=${publicPrefix})`);
			// Run the bundler in-process from the app dir so process.cwd() — and the
			// PYLON_PUBLIC_PREFIX it bakes into the manifest — are the app's.
			await sh(
				[
					"bun",
					"-e",
					'await (await import("@pylonsync/functions/client-bundler")).buildClientBundle();',
				],
				appDir,
			);
			if (!existsSync(`${clientBuildDir}/manifest.json`)) {
				throw new Error("client bundle produced no manifest.json");
			}
			// Collect the content-hashed assets (NOT manifest.json — that stays
			// served same-origin by the app machine for client-side navigation) as
			// a small {files:[{path,b64}]} payload and PUT it to the presigned url;
			// the control plane fans it out to the public CDN bucket.
			const files: Array<{ path: string; b64: string }> = [];
			const walk = async (rel: string): Promise<void> => {
				const dir = rel ? `${clientBuildDir}/${rel}` : clientBuildDir;
				for (const ent of await readdir(dir, { withFileTypes: true })) {
					const childRel = rel ? `${rel}/${ent.name}` : ent.name;
					if (ent.isDirectory()) {
						await walk(childRel);
					} else if (ent.name !== "manifest.json" && ent.name !== ".prebuilt") {
						const buf = await readFile(`${clientBuildDir}/${childRel}`);
						files.push({ path: childRel, b64: buf.toString("base64") });
					}
				}
			};
			await walk("");
			const resp = await fetch(assetsPutUrl, {
				method: "PUT",
				headers: { "content-type": "application/json" },
				body: JSON.stringify({ files }),
			});
			if (!resp.ok) throw new Error(`upload client assets: HTTP ${resp.status}`);
			// Marker LAST: the runtime only reuses a COMPLETE prebuilt bundle.
			await Bun.write(`${clientBuildDir}/.prebuilt`, "1");
			clientAssetsPublished = true;
			console.log(`[builder] published ${files.length} client assets for CDN`);
		} catch (err) {
			console.error(
				`[builder] client pre-build/publish failed (${err}); runtime will build lazily`,
			);
			clientAssetsPublished = false;
			// Drop the partial build so it isn't shipped and the runtime rebuilds
			// cleanly with its own (local-prefix) manifest.
			await rm(clientBuildDir, { recursive: true, force: true }).catch(() => {});
		}
	}

	// 4. Assemble the FULL-PREBUILT bundle: everything the runtime needs at
	//    boot (app.ts, functions/, lib/, node_modules/, web/dist/, public/, …),
	//    minus VCS + dev/build-cache debris. Deterministic-ish: sorted names.
	//    `.pylon` is dropped EXCEPT `.pylon/client-build` — which ships only when
	//    the CDN pre-build above succeeded (else it was removed), carrying the
	//    prebuilt manifest + `.prebuilt` marker the runtime reuses.
	await sh([
		"tar",
		"czf",
		BUNDLE,
		"-C",
		SRC,
		"--exclude=./.git",
		"--exclude=./.github",
		"--exclude=./.pylon/*.db",
		"--exclude=./.pylon/*.db-shm",
		"--exclude=./.pylon/*.db-wal",
		"--exclude=./.pylon/cache",
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
		// True only when the SSR client assets were pre-built + uploaded for the
		// CDN (step 3c). The control plane uses this to decide whether to fan the
		// assets out to the public bucket and point machines at the CDN.
		clientAssets: clientAssetsPublished,
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
