import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";

import {
	exportNames,
	importMap,
	SHARED_MODULES,
	shimSource,
	type SharedSpecifier,
} from "./module-shims";
import { sharedModules } from "./shared-modules";

const specifiers = Object.keys(SHARED_MODULES) as SharedSpecifier[];

/** The names a shim source exports. */
function shimExports(source: string): string[] {
	const list = /export const \{ ([^}]*) \} = m;/.exec(source)?.[1] ?? "";
	return list ? list.split(", ") : [];
}

describe("module shims", () => {
	for (const specifier of specifiers) {
		test(`the "${specifier}" shim exports every export of the real module`, async () => {
			const real = (await import(specifier)) as object;
			const shim = shimExports(
				shimSource(specifier, exportNames(sharedModules()[specifier])),
			);
			const missing = exportNames(real).filter((name) => !shim.includes(name));
			expect(missing).toEqual([]);
		});
	}

	test("the react shim has the hooks an extension uses", () => {
		const shim = shimExports(shimSource("react", exportNames(sharedModules().react)));
		for (const name of ["useState", "useEffect", "createElement", "Fragment"]) {
			expect(shim).toContain(name);
		}
	});

	test("jsxDEV renders with the production jsx", () => {
		const dev = sharedModules()["react/jsx-dev-runtime"] as {
			jsxDEV: (type: string, props: object, key?: string) => { type: string; key: string | null };
		};
		const el = dev.jsxDEV("div", { children: "x" }, "k");
		expect(el.type).toBe("div");
		expect(el.key).toBe("k");
	});

	test("a shim reads the module from the global and exports a default", () => {
		const source = shimSource("react-dom/client", ["createRoot", "hydrateRoot"]);
		expect(source).toContain('globalThis.__PYLON_STUDIO_MODULES__["react-dom/client"]');
		expect(source).toContain("export default m.default ?? m;");
		expect(source).toContain("export const { createRoot, hydrateRoot } = m;");
	});

	test("exportNames drops default and names that are not identifiers", () => {
		expect(exportNames({ default: 1, b: 1, a: 1, "not-valid": 1 })).toEqual(["a", "b"]);
	});

	test("index.html holds the import map, before the module script", () => {
		const html = readFileSync(path.resolve(import.meta.dir, "../../index.html"), "utf8");
		const map = /<script type="importmap">([\s\S]*?)<\/script>/.exec(html);
		expect(map).not.toBeNull();
		expect(JSON.parse(map![1])).toEqual(importMap());
		expect(html.indexOf('type="importmap"')).toBeLessThan(html.indexOf('type="module"'));
	});
});
