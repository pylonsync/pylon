// Entry the Rust CLI runs for `pylon build`:
//
//   bun run <@pylonsync/functions>/src/build-cli.ts --out <dir>
//     [--app-dir app] [--manifest <path to pylon.manifest.json>]
//
// Writes the artifact (see production-build.ts) and prints one JSON line with
// the result on stdout. Errors go to stderr with a non-zero exit.

import { buildProduction } from "./production-build";

function flag(name: string): string | undefined {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

// The React production build, and the same NODE_ENV the server bundle
// defines. Set before the bundler reads it.
process.env.NODE_ENV = "production";

const path = await import("node:path");
const cwd = process.cwd();
const out = flag("--out") ?? "dist";

try {
  const result = await buildProduction({
    cwd,
    outDir: path.resolve(cwd, out),
    appDir: flag("--app-dir") ?? "app",
    manifestPath: flag("--manifest"),
    functionsDir: process.env.PYLON_FUNCTIONS_DIR ?? "functions",
  });
  process.stdout.write(JSON.stringify({ ok: true, ...result }) + "\n");
} catch (e: any) {
  process.stderr.write(`${e?.message ?? String(e)}\n`);
  process.exit(1);
}
