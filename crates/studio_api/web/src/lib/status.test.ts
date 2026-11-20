import { describe, expect, test } from "bun:test";
import { isServerError, statusClass } from "./status";

describe("statusClass", () => {
	test("1xx to 3xx is a success", () => {
		for (const s of [101, 200, 204, 301, 399]) expect(statusClass(s)).toBe("ok");
	});

	test("4xx is a client error, not a server error", () => {
		for (const s of [400, 401, 404, 429, 499]) {
			expect(statusClass(s)).toBe("client");
			expect(isServerError(s)).toBe(false);
		}
	});

	test("5xx and a status that is not valid HTTP are server errors", () => {
		for (const s of [500, 502, 599, 0, 99]) expect(isServerError(s)).toBe(true);
	});
});
