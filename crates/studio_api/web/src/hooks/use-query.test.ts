import { expect, test } from "bun:test";

import { nextRequestNumber, responseReplaces } from "./use-query";

test("an older response for the same key does not replace newer data", () => {
	const shown = { key: "list", request: 7 };
	expect(responseReplaces(shown, { key: "list", request: 5 })).toBe(false);
	expect(responseReplaces(shown, { key: "list", request: 8 })).toBe(true);
});

test("a response for another key replaces the shown data", () => {
	expect(responseReplaces({ key: "a", request: 9 }, { key: "b", request: 2 })).toBe(true);
	expect(responseReplaces({ key: null, request: 0 }, { key: "a", request: 1 })).toBe(true);
});

test("the next request number is larger than any request so far", () => {
	const n = nextRequestNumber();
	expect(n).toBeGreaterThan(0);
	expect(nextRequestNumber()).toBe(n);
});
