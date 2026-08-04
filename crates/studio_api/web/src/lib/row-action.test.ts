import { describe, expect, test } from "bun:test";
import {
	buildActionInput,
	getRowField,
	pickResultValue,
	resultToText,
} from "./row-action";
import type { RowAction } from "./studio-config";

const base: RowAction = { id: "go", label: "Go", kind: "action" };
const row = {
	id: "prop_1",
	title: "Q3 rewrite",
	count: 3,
	draft: false,
	owner: { email: "a@b.c" },
	missing: null,
};

describe("getRowField", () => {
	test("reads a flat field", () => {
		expect(getRowField(row, "title")).toBe("Q3 rewrite");
	});

	test("reads a dotted path", () => {
		expect(getRowField(row, "owner.email")).toBe("a@b.c");
	});

	test("returns undefined for a missing segment instead of throwing", () => {
		expect(getRowField(row, "owner.name.first")).toBeUndefined();
		expect(getRowField(row, "nope")).toBeUndefined();
	});
});

describe("buildActionInput", () => {
	test("defaults to the row id", () => {
		expect(buildActionInput(base, row)).toEqual({ id: "prop_1" });
	});

	test("sends a null id rather than omitting it when the row has none", () => {
		expect(buildActionInput(base, { title: "x" })).toEqual({ id: null });
	});

	test("substitutes a whole-value placeholder", () => {
		const a: RowAction = { ...base, input: { proposalId: "{row.id}" } };
		expect(buildActionInput(a, row)).toEqual({ proposalId: "prop_1" });
	});

	test("keeps the JSON type of a whole-value placeholder", () => {
		const a: RowAction = {
			...base,
			input: { n: "{row.count}", flag: "{row.draft}", who: "{row.owner}" },
		};
		expect(buildActionInput(a, row)).toEqual({
			n: 3,
			flag: false,
			who: { email: "a@b.c" },
		});
	});

	test("a missing whole-value placeholder becomes null, not the literal", () => {
		const a: RowAction = { ...base, input: { x: "{row.nothere}" } };
		expect(buildActionInput(a, row)).toEqual({ x: null });
	});

	test("interpolates a placeholder embedded in a larger string", () => {
		const a: RowAction = { ...base, input: { slug: "p/{row.id}/{row.count}" } };
		expect(buildActionInput(a, row)).toEqual({ slug: "p/prop_1/3" });
	});

	test("a missing embedded placeholder collapses to empty, never 'undefined'", () => {
		const a: RowAction = { ...base, input: { s: "a{row.nothere}b" } };
		expect(buildActionInput(a, row)).toEqual({ s: "ab" });
	});

	test("passes non-string literals through untouched", () => {
		const a: RowAction = { ...base, input: { limit: 10, on: true, z: null } };
		expect(buildActionInput(a, row)).toEqual({ limit: 10, on: true, z: null });
	});

	test("recurses into nested objects and arrays", () => {
		const a: RowAction = {
			...base,
			input: { opts: { to: "{row.owner.email}" }, ids: ["{row.id}", "static"] },
		};
		expect(buildActionInput(a, row)).toEqual({
			opts: { to: "a@b.c" },
			ids: ["prop_1", "static"],
		});
	});

	test("leaves a string with no placeholder alone", () => {
		const a: RowAction = { ...base, input: { mode: "preview" } };
		expect(buildActionInput(a, row)).toEqual({ mode: "preview" });
	});
});

describe("pickResultValue", () => {
	test("returns the whole value with no field", () => {
		expect(pickResultValue({ url: "u" })).toEqual({ url: "u" });
	});

	test("reads a field", () => {
		expect(pickResultValue({ url: "u" }, "url")).toBe("u");
	});

	test("reads a nested field", () => {
		expect(pickResultValue({ a: { b: 1 } }, "a.b")).toBe(1);
	});

	test("returns undefined when the result isn't an object", () => {
		expect(pickResultValue("plain", "url")).toBeUndefined();
		expect(pickResultValue(null, "url")).toBeUndefined();
	});
});

describe("resultToText", () => {
	test("strings pass through unquoted so a URL stays pasteable", () => {
		expect(resultToText("https://x.test/a")).toBe("https://x.test/a");
	});

	test("null and undefined render empty", () => {
		expect(resultToText(null)).toBe("");
		expect(resultToText(undefined)).toBe("");
	});

	test("objects render as pretty JSON", () => {
		expect(resultToText({ a: 1 })).toBe('{\n  "a": 1\n}');
	});

	test("numbers and booleans stringify", () => {
		expect(resultToText(42)).toBe("42");
		expect(resultToText(false)).toBe("false");
	});
});
