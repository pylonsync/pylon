// Write dist/modules/<shim>.js for each module that Studio shares with
// extensions. Runs after `vite build`. The Pylon binary embeds the files and
// serves them under /studio/modules/.

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

import {
	exportNames,
	SHARED_MODULES,
	shimSource,
	type SharedSpecifier,
} from "../src/lib/module-shims";
import { sharedModules } from "../src/lib/shared-modules";

const outDir = path.resolve(import.meta.dirname, "../dist/modules");
mkdirSync(outDir, { recursive: true });

const modules = sharedModules();
for (const [specifier, file] of Object.entries(SHARED_MODULES)) {
	const names = exportNames(modules[specifier as SharedSpecifier]);
	writeFileSync(
		path.join(outDir, file),
		shimSource(specifier as SharedSpecifier, names),
	);
	console.log(`dist/modules/${file}  ${names.length} exports`);
}
