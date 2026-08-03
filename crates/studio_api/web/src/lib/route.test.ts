import { describe, expect, test } from "bun:test";
import { DEFAULT_PAGE, parseRoute, routeToPath, sameRoute } from "./route";

// Studio v1 held the route in React state and never wrote the URL, so none of
// this existed: no deep links, refresh always landed on Overview, and back
// left Studio. These pin the scheme that replaced it.

describe("routeToPath", () => {
	test("pages and entities live in separate namespaces", () => {
		expect(routeToPath({ kind: "page", id: "health" })).toBe("/studio/health");
		expect(routeToPath({ kind: "resource", entity: "User" })).toBe(
			"/studio/e/User",
		);
	});

	test("an entity named like a page does not shadow it", () => {
		// The `e/` prefix is the whole reason this can't collide. Without it,
		// an app with a `settings` entity would fight the Settings page and the
		// winner would depend on match order.
		const page = routeToPath({ kind: "page", id: "settings" });
		const entity = routeToPath({ kind: "resource", entity: "settings" });
		expect(page).not.toBe(entity);
		expect(parseRoute(page)).toEqual({ kind: "page", id: "settings" });
		expect(parseRoute(entity)).toEqual({ kind: "resource", entity: "settings" });
	});

	test("encodes names that would otherwise break the path", () => {
		expect(routeToPath({ kind: "resource", entity: "a/b" })).toBe(
			"/studio/e/a%2Fb",
		);
	});
});

describe("parseRoute", () => {
	test("round-trips every route shape", () => {
		const routes = [
			{ kind: "page", id: "overview" },
			{ kind: "page", id: "custom-dashboard" },
			{ kind: "resource", entity: "User" },
			{ kind: "resource", entity: "a/b" },
			{ kind: "resource", entity: "Ünïcode" },
		] as const;
		for (const r of routes) {
			expect(parseRoute(routeToPath(r))).toEqual(r);
		}
	});

	test("bare /studio yields null so the caller picks its default", () => {
		// Not hard-coded to Overview here: an app whose sidebar config declares
		// a first page expects to land on that instead.
		expect(parseRoute("/studio")).toBeNull();
		expect(parseRoute("/studio/")).toBeNull();
	});

	test("query and hash are ignored", () => {
		expect(parseRoute("/studio/health?tab=1#x")).toEqual({
			kind: "page",
			id: "health",
		});
	});

	test("trailing slashes are tolerated", () => {
		expect(parseRoute("/studio/e/User/")).toEqual({
			kind: "resource",
			entity: "User",
		});
	});

	test("extra segments land on the page rather than nothing", () => {
		expect(parseRoute("/studio/overview/stray/bits")).toEqual({
			kind: "page",
			id: "overview",
		});
	});

	test("`/studio/e` with no entity is not a route", () => {
		// Rendering a list for the empty-string entity would fire a doomed
		// request; falling back to the default page is the useful answer.
		expect(parseRoute("/studio/e")).toBeNull();
		expect(parseRoute("/studio/e/")).toBeNull();
	});

	test("non-Studio paths are not Studio routes", () => {
		// `/studios` must not parse as Studio — the same prefix bug that once
		// made the server swallow app routes like /studios and /eventsfeed.
		for (const p of ["/", "/studios", "/studiox/y", "/api/studio"]) {
			expect(parseRoute(p)).toBeNull();
		}
	});

	test("a malformed percent escape does not throw", () => {
		// decodeURIComponent("%E0%A4%A") throws URIError. Uncaught during the
		// initial parse it would blank the whole app.
		expect(() => parseRoute("/studio/e/%E0%A4%A")).not.toThrow();
		expect(parseRoute("/studio/e/%E0%A4%A")).toEqual({
			kind: "resource",
			entity: "%E0%A4%A",
		});
	});
});

describe("sameRoute", () => {
	test("distinguishes a page from an entity of the same name", () => {
		expect(
			sameRoute({ kind: "page", id: "User" }, { kind: "resource", entity: "User" }),
		).toBe(false);
	});

	test("matches identical routes and separates different ones", () => {
		expect(
			sameRoute({ kind: "page", id: "health" }, { kind: "page", id: "health" }),
		).toBe(true);
		expect(
			sameRoute({ kind: "page", id: "health" }, { kind: "page", id: "sync" }),
		).toBe(false);
		expect(
			sameRoute(
				{ kind: "resource", entity: "User" },
				{ kind: "resource", entity: "Post" },
			),
		).toBe(false);
	});
});

describe("defaults", () => {
	test("the fallback page is one Studio actually ships", () => {
		expect(DEFAULT_PAGE).toBe("overview");
	});
});
