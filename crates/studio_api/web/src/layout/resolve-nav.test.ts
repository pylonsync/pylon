import { describe, expect, test } from "bun:test";

import type { Manifest } from "@/lib/pylon";
import { resolveNav } from "./resolve-nav";

const manifest = {
	manifest_version: 1,
	name: "app",
	version: "1.0.0",
	entities: [{ name: "Deal", fields: [] }],
	policies: [],
	queries: [],
	actions: [],
	routes: [],
} as unknown as Manifest;

describe("default sidebar", () => {
	test("Overview is the first item, above the Data group", () => {
		const sections = resolveNav({}, manifest);
		expect(sections[0].label).toBe("");
		expect(sections[0].items).toEqual([
			{ kind: "page", id: "overview", label: "Overview", icon: "bar-chart" },
		]);
		expect(sections[1].label).toBe("Data");
	});

	test("Overview appears once", () => {
		const overview = resolveNav({}, manifest)
			.flatMap((s) => s.items)
			.filter((i) => i.kind === "page" && i.id === "overview");
		expect(overview).toHaveLength(1);
	});
});
