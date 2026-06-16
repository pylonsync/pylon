#!/usr/bin/env node
/**
 * Template dev harness — iterate on a create-pylon template without
 * re-scaffolding by hand.
 *
 *   node packages/create-pylon/dev.mjs <template> [port]
 *   node packages/create-pylon/dev.mjs default        # the default template, port 4321
 *   node packages/create-pylon/dev.mjs todo 4480
 *
 * What it does:
 *   1. Substitutes the template's placeholders and copies it into a runnable
 *      work dir (default: <os-tmp>/pylon-tpl-dev/<template>), installs deps
 *      once (cached across runs).
 *   2. Watches the real template dir and re-syncs any file you change into the
 *      work dir — so `pylon dev`'s hot-reload picks up your edits live.
 *   3. Runs `pylon dev` in the work dir.
 *
 * Edit files under packages/create-pylon/templates/<template>/ and the running
 * app reloads. This is a maintainer tool — it is NOT shipped in the npm package
 * (only `bin` + `templates` are published).
 */

import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  watch,
  writeFileSync,
  statSync,
} from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { spawn, spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const TEMPLATES = resolve(HERE, "templates");
const PKG_VERSION = JSON.parse(
  readFileSync(resolve(HERE, "package.json"), "utf8"),
).version;

const [, , template, portArg] = process.argv;
const port = portArg || "4321";

if (!template) {
  console.error(
    "usage: node packages/create-pylon/dev.mjs <template> [port]\n" +
      "       template is a unified template dir name (default, todo, barebones, consumer, chat, b2b, waitlist, local-service, restaurant, creator, shop, agency, marketplace)",
  );
  process.exit(1);
}

const srcDir = join(TEMPLATES, template);
if (!existsSync(join(srcDir, "app.ts"))) {
  console.error(
    `error: ${srcDir} is not a runnable unified template (no app.ts). ` +
      `This harness only runs single-app templates.`,
  );
  process.exit(1);
}

const workDir = join(tmpdir(), "pylon-tpl-dev", template);

// Placeholder substitution — mirror the scaffolder's so the dev copy matches a
// real `npm create` output (minus interactive bits).
const APP = `${template}-dev`;
function substitute(text) {
  return text
    .replaceAll("__APP_NAME__", APP)
    .replaceAll("__APP_NAME_KEBAB__", APP)
    .replaceAll("__APP_NAME_SNAKE__", APP.replaceAll("-", "_"))
    .replaceAll("__PYLON_VERSION__", PKG_VERSION)
    .replaceAll("__RUN_DEV__", "pylon dev");
}

const TEXT_EXT = new Set([
  ".ts",
  ".tsx",
  ".js",
  ".mjs",
  ".json",
  ".css",
  ".md",
  ".html",
  ".txt",
]);

// Copy one template file → work dir, substituting placeholders in text files
// and renaming `gitignore` → `.gitignore` (npm strips dotfiles from packages,
// so templates ship it un-dotted).
function syncFile(rel) {
  const from = join(srcDir, rel);
  if (!existsSync(from) || statSync(from).isDirectory()) return;
  const destRel = rel === "gitignore" ? ".gitignore" : rel;
  const to = join(workDir, destRel);
  mkdirSync(dirname(to), { recursive: true });
  const ext = destRel.slice(destRel.lastIndexOf("."));
  if (TEXT_EXT.has(ext) || destRel === ".gitignore") {
    writeFileSync(to, substitute(readFileSync(from, "utf8")));
  } else {
    cpSync(from, to);
  }
}

// Initial full copy: walk every template file through syncFile (which applies
// substitution + the gitignore rename). Empty dirs are irrelevant.
function fullSync() {
  walk(srcDir);
}
function walk(dir) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === ".pylon") continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full);
    else syncFile(relative(srcDir, full));
  }
}

const firstRun = !existsSync(join(workDir, "node_modules"));
console.log(
  `[tpl-dev] ${template} → ${workDir}${firstRun ? " (first run: install)" : ""}`,
);
rmSync(join(workDir, "app"), { recursive: true, force: true });
rmSync(join(workDir, "components"), { recursive: true, force: true });
mkdirSync(workDir, { recursive: true });
fullSync();

if (firstRun) {
  const r = spawnSync("bun", ["install"], { cwd: workDir, stdio: "inherit" });
  if (r.status !== 0) {
    console.error("[tpl-dev] bun install failed");
    process.exit(1);
  }
}

// Watch the template and re-sync changed files live.
let timer = null;
const pending = new Set();
const watcher = watch(srcDir, { recursive: true }, (_event, filename) => {
  if (!filename) return;
  const base = filename.split(sep).pop();
  if (base === "node_modules" || base === ".pylon") return;
  pending.add(filename);
  clearTimeout(timer);
  timer = setTimeout(() => {
    for (const rel of pending) {
      try {
        syncFile(rel);
        console.log(`[tpl-dev] synced ${rel}`);
      } catch {
        /* file may have been removed mid-write; ignore */
      }
    }
    pending.clear();
  }, 80);
});

console.log(`[tpl-dev] watching ${srcDir} — edit and save to hot-reload`);
const dev = spawn("pylon", ["dev", "--port", port], {
  cwd: workDir,
  stdio: "inherit",
});

function shutdown() {
  watcher.close();
  dev.kill("SIGINT");
  process.exit(0);
}
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
dev.on("exit", (code) => {
  watcher.close();
  process.exit(code ?? 0);
});
