import { describe, expect, test } from "bun:test";
import {
	formatDuration,
	formatUptime,
	humanize,
	isNumericType,
	parseDate,
	percentile,
	plural,
	routeOfPath,
	shortId,
} from "./format";

describe("shortId", () => {
	test("keeps a short id whole", () => {
		expect(shortId("user_1")).toBe("user_1");
		expect(shortId("0123456789")).toBe("0123456789");
	});

	test("shows the first 8 characters of a long id", () => {
		expect(shortId("9b2f1c3e-77aa-4c1d-8e21-aabbccddeeff")).toBe("9b2f1c3e…");
	});

	test("shows the last 12 characters of an id that starts with zeros", () => {
		expect(shortId("000000000000000018e982dbb712c6780000003f")).toBe("…c6780000003f");
	});
});

describe("humanize", () => {
	test("splits camel case and separators", () => {
		expect(humanize("createdAt")).toBe("Created at");
		expect(humanize("close_date")).toBe("Close date");
		expect(humanize("id")).toBe("Id");
	});
});

describe("plural", () => {
	test("picks the form for the count", () => {
		expect(plural(1, "row")).toBe("1 row");
		expect(plural(0, "row")).toBe("0 rows");
		expect(plural(2, "policy", "policies")).toBe("2 policies");
	});
});

describe("formatUptime", () => {
	test("uses the two largest units", () => {
		expect(formatUptime(14)).toBe("14s");
		expect(formatUptime(125)).toBe("2m");
		expect(formatUptime(3 * 3600 + 120)).toBe("3h 2m");
		expect(formatUptime(2 * 86_400 + 3600)).toBe("2d 1h");
	});
});

describe("isNumericType", () => {
	test("is true for int and float only", () => {
		expect(isNumericType("int")).toBe(true);
		expect(isNumericType("float")).toBe(true);
		expect(isNumericType("string")).toBe(false);
		expect(isNumericType(undefined)).toBe(false);
	});
});

describe("parseDate", () => {
	test("reads an ISO string", () => {
		expect(parseDate("2026-11-19T18:20:02.000Z")?.toISOString()).toBe(
			"2026-11-19T18:20:02.000Z",
		);
	});

	test("reads a Unix time in seconds or milliseconds", () => {
		expect(parseDate(1795112402)?.toISOString()).toBe("2026-11-19T18:20:02.000Z");
		expect(parseDate(1795112402000)?.toISOString()).toBe(
			"2026-11-19T18:20:02.000Z",
		);
		expect(parseDate("1795112402Z")?.toISOString()).toBe(
			"2026-11-19T18:20:02.000Z",
		);
	});

	test("returns null for a value that is not a date", () => {
		expect(parseDate("not a date")).toBeNull();
		expect(parseDate("")).toBeNull();
		expect(parseDate(null)).toBeNull();
		expect(parseDate(Number.NaN)).toBeNull();
	});
});

describe("formatDuration", () => {
	test("picks the unit for the size", () => {
		expect(formatDuration(0.42)).toBe("0.4 ms");
		expect(formatDuration(12.4)).toBe("12 ms");
		expect(formatDuration(1400)).toBe("1.4 s");
		expect(formatDuration(125_000)).toBe("2m 5s");
	});
});

describe("percentile", () => {
	test("returns the value at the rank", () => {
		const sorted = [1, 2, 3, 4, 5, 6, 7, 8, 9, 100];
		expect(percentile(sorted, 0.5)).toBe(5);
		expect(percentile(sorted, 0.95)).toBe(100);
		expect(percentile(sorted, 1)).toBe(100);
		expect(percentile([7], 0.99)).toBe(7);
	});

	test("returns null for no values", () => {
		expect(percentile([], 0.5)).toBeNull();
	});
});

describe("routeOfPath", () => {
	test("replaces id segments", () => {
		expect(
			routeOfPath("/api/entities/Deal/000000000000000018e982dbb712c6780000003f"),
		).toBe("/api/entities/Deal/:id");
		expect(routeOfPath("/api/workflows/wf_pylon-8a26a59a_3/cancel")).toBe(
			"/api/workflows/:id/cancel",
		);
		expect(routeOfPath("/deals/42")).toBe("/deals/:id");
	});

	test("keeps named segments", () => {
		expect(routeOfPath("/api/entities/Deal")).toBe("/api/entities/Deal");
		expect(routeOfPath("/api/fn/moveDeal")).toBe("/api/fn/moveDeal");
		expect(routeOfPath("/internal/components")).toBe("/internal/components");
		expect(routeOfPath("/")).toBe("/");
	});
});
