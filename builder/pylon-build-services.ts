// Packs the services an app declares (`buildManifest({ services })`) for the
// control plane to deploy beside the app.
//
// Each service's source directory goes into one archive, `services.tar.gz`,
// under a top-level directory named after the service. The COMPLETE marker
// then carries each declaration with a hash of its files, so the control
// plane redeploys a service only when its source or its declaration changed.

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { lstat, mkdir, readFile, readdir, readlink, realpath } from "node:fs/promises";
import { isAbsolute, relative, sep } from "node:path";

/** Directory and file names left out of a service's archive, at any depth. */
export const SERVICE_EXCLUDES = [
	".git",
	"node_modules",
	".venv",
	"__pycache__",
	".pytest_cache",
	".mypy_cache",
	".ruff_cache",
	".DS_Store",
];

/** The largest services archive the builder uploads. */
export const MAX_SERVICES_ARCHIVE_BYTES = 100 * 1024 * 1024;

/** A service as the manifest declares it. The control plane validates it. */
export type ServiceDeclaration = Record<string, unknown> & { name: string; dir: string };

export interface PackedService {
	/** The manifest entry, as written. */
	declaration: ServiceDeclaration;
	/** SHA-256 over the service's files: paths, modes and contents. */
	sourceHash: string;
}

const NAME = /^[a-z][a-z0-9-]{0,30}$/;

/** The manifest's services, or [] when it declares none. Throws on an entry
 *  whose name or directory cannot be packed. */
export function declaredServices(manifest: unknown): ServiceDeclaration[] {
	const list = (manifest as { services?: unknown } | null)?.services;
	if (list === undefined) return [];
	if (!Array.isArray(list)) throw new Error("manifest services must be a list");
	const seen = new Set<string>();
	return list.map((entry) => {
		const def = entry as ServiceDeclaration;
		if (typeof def?.name !== "string" || !NAME.test(def.name)) {
			throw new Error(`service name ${JSON.stringify(def?.name)} is not valid`);
		}
		if (seen.has(def.name)) throw new Error(`two services are named "${def.name}"`);
		seen.add(def.name);
		const parts = typeof def.dir === "string" ? def.dir.replace(/\\/g, "/").split("/") : [];
		const real = parts.filter((p) => p !== "" && p !== ".");
		if (real.length === 0 || parts[0] === "" || parts.includes("..") || /^[A-Za-z]:/.test(parts[0])) {
			throw new Error(`service "${def.name}": dir must be a directory below the app root`);
		}
		return def;
	});
}

/**
 * SHA-256 over every file below `dir`, in path order: the relative path, the
 * executable bit, and the contents (a symlink's target in place of contents).
 * The same files give the same hash on any machine; an archive's own hash
 * does not, because gzip and tar record times.
 */
export async function hashDirectory(dir: string): Promise<string> {
	const hash = createHash("sha256");
	const walk = async (rel: string): Promise<void> => {
		const entries = await readdir(rel ? `${dir}/${rel}` : dir, { withFileTypes: true });
		entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
		for (const ent of entries) {
			if (SERVICE_EXCLUDES.includes(ent.name)) continue;
			const childRel = rel ? `${rel}/${ent.name}` : ent.name;
			const full = `${dir}/${childRel}`;
			if (ent.isDirectory()) {
				await walk(childRel);
			} else if (ent.isSymbolicLink()) {
				hash.update(`l\0${childRel}\0${await readlink(full)}\0`);
			} else if (ent.isFile()) {
				const exec = ((await lstat(full)).mode & 0o111) !== 0;
				const body = await readFile(full);
				hash.update(`f\0${childRel}\0${exec ? "x" : "-"}\0${body.length}\0`);
				hash.update(body);
			}
		}
	};
	await walk("");
	return hash.digest("hex");
}

/**
 * Stage every declared service under `stageDir/<name>` and write them to
 * `archivePath`. `run` executes a command and throws when it fails.
 */
export async function packServices(opts: {
	appDir: string;
	services: ServiceDeclaration[];
	stageDir: string;
	archivePath: string;
	run: (cmd: string[], cwd?: string) => Promise<void>;
}): Promise<PackedService[]> {
	const packed: PackedService[] = [];
	const appRoot = await realpath(opts.appDir);
	await mkdir(opts.stageDir, { recursive: true });
	for (const declaration of opts.services) {
		const source = `${opts.appDir}/${declaration.dir.replace(/\\/g, "/")}`;
		if (!existsSync(source) || !(await lstat(source)).isDirectory()) {
			throw new Error(`service "${declaration.name}": no directory at ${declaration.dir}`);
		}
		const sourceDir = await realpath(source);
		const pathFromRoot = relative(appRoot, sourceDir);
		if (!pathFromRoot || pathFromRoot === ".." || pathFromRoot.startsWith(`..${sep}`) || isAbsolute(pathFromRoot)) {
			throw new Error(`service "${declaration.name}": dir must be a directory below the app root`);
		}
		const target = `${opts.stageDir}/${declaration.name}`;
		await mkdir(target, { recursive: true });
		const excludes = SERVICE_EXCLUDES.map((name) => `--exclude=${name}`);
		// tar to tar keeps modes and symlinks and applies the excludes.
		// bash, for "${@:3}".
		await opts.run(["bash", "-c", 'tar -C "$1" "${@:3}" -cf - . | tar -C "$2" -xf -', "bash", sourceDir, target, ...excludes]);
		packed.push({ declaration, sourceHash: await hashDirectory(target) });
	}
	await opts.run(["tar", "czf", opts.archivePath, "-C", opts.stageDir, "."]);
	return packed;
}
