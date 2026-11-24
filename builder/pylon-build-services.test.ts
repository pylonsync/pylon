import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, chmodSync, existsSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { declaredServices, hashDirectory, packServices } from "./pylon-build-services";

const root = mkdtempSync(join(tmpdir(), "pylon-builder-services-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

async function run(cmd: string[], cwd?: string): Promise<void> {
	const proc = Bun.spawn(cmd, { cwd, stdout: "ignore", stderr: "pipe" });
	if ((await proc.exited) !== 0) throw new Error(await new Response(proc.stderr).text());
}

function app(name: string): string {
	const dir = join(root, name);
	mkdirSync(join(dir, "services/worker/pkg"), { recursive: true });
	mkdirSync(join(dir, "services/worker/.venv/bin"), { recursive: true });
	mkdirSync(join(dir, "services/worker/pkg/__pycache__"), { recursive: true });
	writeFileSync(join(dir, "services/worker/pkg/app.py"), "print('hi')\n");
	writeFileSync(join(dir, "services/worker/run.sh"), "#!/bin/sh\n");
	chmodSync(join(dir, "services/worker/run.sh"), 0o755);
	writeFileSync(join(dir, "services/worker/.venv/bin/python"), "x".repeat(1000));
	writeFileSync(join(dir, "services/worker/pkg/__pycache__/app.pyc"), "cache");
	return dir;
}

test("declaredServices reads the manifest and refuses entries it cannot pack", () => {
	expect(declaredServices({})).toEqual([]);
	expect(declaredServices({ services: [{ name: "worker", dir: "services/worker" }] })).toHaveLength(1);
	expect(() => declaredServices({ services: {} })).toThrow("must be a list");
	expect(() => declaredServices({ services: [{ name: "Bad", dir: "a" }] })).toThrow("not valid");
	expect(() => declaredServices({ services: [{ name: "a", dir: "../a" }] })).toThrow("below the app root");
	expect(() => declaredServices({ services: [{ name: "a", dir: "/etc" }] })).toThrow("below the app root");
	expect(() => declaredServices({ services: [{ name: "a", dir: "." }] })).toThrow("below the app root");
	expect(() => declaredServices({ services: [{ name: "a", dir: "a" }, { name: "a", dir: "b" }] })).toThrow("two services");
});

test("the hash follows the files, not the machine or the excluded directories", async () => {
	const a = app("a");
	const b = app("b");
	const hashA = await hashDirectory(join(a, "services/worker"));
	expect(hashA).toBe(await hashDirectory(join(b, "services/worker")));
	writeFileSync(join(b, "services/worker/.venv/bin/python"), "changed");
	expect(await hashDirectory(join(b, "services/worker"))).toBe(hashA);
	writeFileSync(join(b, "services/worker/pkg/app.py"), "print('bye')\n");
	expect(await hashDirectory(join(b, "services/worker"))).not.toBe(hashA);
	chmodSync(join(a, "services/worker/run.sh"), 0o644);
	expect(await hashDirectory(join(a, "services/worker"))).not.toBe(hashA);
});

test("packServices stages each service under its name without the excluded directories", async () => {
	const dir = app("c");
	symlinkSync(join(dir, "services"), join(dir, "service-alias"), "dir");
	const stageDir = join(root, "stage-c");
	const archivePath = join(root, "services-c.tar.gz");
	const packed = await packServices({
		appDir: dir,
		services: [{ name: "worker", dir: "service-alias/worker" }],
		stageDir,
		archivePath,
		run,
	});
	expect(packed).toHaveLength(1);
	expect(packed[0].sourceHash).toBe(await hashDirectory(join(dir, "services/worker")));
	expect(existsSync(join(stageDir, "worker/pkg/app.py"))).toBe(true);
	expect(existsSync(join(stageDir, "worker/.venv"))).toBe(false);
	expect(existsSync(join(stageDir, "worker/pkg/__pycache__"))).toBe(false);
	const out = join(root, "out-c");
	mkdirSync(out);
	await run(["tar", "xzf", archivePath, "-C", out]);
	expect(existsSync(join(out, "worker/run.sh"))).toBe(true);
});

test("a missing directory fails the build", async () => {
	const dir = app("d");
	await expect(
		packServices({ appDir: dir, services: [{ name: "gone", dir: "services/gone" }], stageDir: join(root, "stage-d"), archivePath: join(root, "d.tar.gz"), run }),
	).rejects.toThrow("no directory at services/gone");
});

test("service packaging rejects a source outside the app before reading its files", async () => {
	const dir = app("linked");
	const sibling = app("linked-sibling");
	symlinkSync(sibling, join(dir, "alias"), "dir");
	let commandCount = 0;
	await expect(packServices({
		appDir: dir,
		services: [{ name: "worker", dir: "alias/services/worker" }],
		stageDir: join(root, "stage-linked"),
		archivePath: join(root, "linked.tar.gz"),
		run: async () => { commandCount++; },
	})).rejects.toThrow("below the app root");
	expect(commandCount).toBe(0);
});
