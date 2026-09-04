import { describe, expect, test } from "bun:test";
import { ACCOUNT_ORIGIN, accountUrl, ctaUrl } from "./account-urls";

// These components compile into two apps on two hosts:
//   apps/control-plane   -> cloud.stack0.dev  (owns /signup and /login)
//   apps/pylonsync-site  -> www.pylonsync.com     (has no auth at all)
//
// Every auth link used to be a relative path, correct only in the first. On
// pylonsync.com the header CTA and every in-page "Create your account" pointed
// at /signup on a host that 404s it.
//
// A relative href reintroduced anywhere here breaks silently — it keeps working
// on cloud.stack0.dev, which is where it would be tested.

describe("account URLs are absolute", () => {
	test("points at the product host, not the framework site", () => {
		expect(ACCOUNT_ORIGIN).toBe("https://cloud.stack0.dev");
	});

	test("signup and login resolve to the account host", () => {
		expect(accountUrl("/signup")).toBe("https://cloud.stack0.dev/signup");
		expect(accountUrl("/login")).toBe("https://cloud.stack0.dev/login");
	});

	test("query strings survive, so plan preselect still works", () => {
		// Stack0 Cloud's plan cards link across with ?plan=; dropping the query
		// would land every visitor on the default plan regardless of choice.
		expect(accountUrl("/signup?plan=team")).toBe(
			"https://cloud.stack0.dev/signup?plan=team",
		);
	});

	test("a path missing its leading slash does not concatenate into the host", () => {
		// "signup" would otherwise produce ...cloud.stack0.devsignup
		expect(accountUrl("signup")).toBe("https://cloud.stack0.dev/signup");
	});

	test("never emits a relative href", () => {
		for (const p of ["/signup", "/login", "/dashboard", "/signup?plan=starter"]) {
			expect(accountUrl(p).startsWith("https://")).toBe(true);
		}
	});
});

describe("ctaUrl", () => {
	test("signed out goes to signup", () => {
		expect(ctaUrl(false)).toBe("https://cloud.stack0.dev/signup");
	});

	test("signed in goes to the dashboard", () => {
		expect(ctaUrl(true)).toBe("https://cloud.stack0.dev/dashboard");
	});

	test("a plan-specific signup is used only when signed out", () => {
		// Carrying ?plan= into /dashboard would be meaningless, and worse, would
		// suggest the plan was applied.
		expect(ctaUrl(false, "/signup?plan=team")).toBe(
			"https://cloud.stack0.dev/signup?plan=team",
		);
		expect(ctaUrl(true, "/signup?plan=team")).toBe(
			"https://cloud.stack0.dev/dashboard",
		);
	});
});
