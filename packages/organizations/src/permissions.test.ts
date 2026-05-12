import { describe, expect, test } from "bun:test";

import { hasPermission, requirePermission } from "./permissions";
import type { HandlerCtx, OrganizationsConfig } from "./types";

const cfg: OrganizationsConfig = {
	permissions: {
		"projects.create": ["owner", "admin"],
		"projects.delete": ["owner"],
		"billing.manage": ["owner"],
	},
};

function fakeCtx(opts: {
	userId?: string;
	tenantId?: string;
	isAdmin?: boolean;
	role?: string;
}): HandlerCtx {
	return {
		env: {},
		auth: {
			userId: opts.userId ?? null,
			tenantId: opts.tenantId ?? null,
			isAdmin: opts.isAdmin ?? false,
		},
		runQuery: async () =>
			(opts.role ? [{ role: opts.role }] : []) as unknown as never,
		runMutation: async () => undefined as unknown as never,
		error: (code, msg) => Object.assign(new Error(`${code}: ${msg}`), { code }),
	};
}

describe("hasPermission", () => {
	test("admin bypasses every check", async () => {
		const ctx = fakeCtx({ isAdmin: true });
		expect(await hasPermission(ctx, cfg, "projects.delete")).toBe(true);
		expect(await hasPermission(ctx, cfg, "missing.unknown")).toBe(true);
	});

	test("anonymous denied", async () => {
		const ctx = fakeCtx({});
		expect(await hasPermission(ctx, cfg, "projects.create")).toBe(false);
	});

	test("owner granted; member denied", async () => {
		const ownerCtx = fakeCtx({
			userId: "u_1",
			tenantId: "o_1",
			role: "owner",
		});
		const memberCtx = fakeCtx({
			userId: "u_2",
			tenantId: "o_1",
			role: "member",
		});
		expect(await hasPermission(ownerCtx, cfg, "projects.delete")).toBe(true);
		expect(await hasPermission(memberCtx, cfg, "projects.delete")).toBe(false);
	});

	test("unknown permission denied", async () => {
		const ctx = fakeCtx({ userId: "u_1", tenantId: "o_1", role: "owner" });
		expect(await hasPermission(ctx, cfg, "secret.action")).toBe(false);
	});
});

describe("requirePermission", () => {
	test("throws UNAUTHENTICATED when no user", async () => {
		const ctx = fakeCtx({});
		await expect(
			requirePermission(ctx, cfg, "projects.create"),
		).rejects.toThrow();
	});

	test("throws UNKNOWN_PERMISSION for undeclared permission", async () => {
		const ctx = fakeCtx({ userId: "u_1", tenantId: "o_1", role: "owner" });
		await expect(
			requirePermission(ctx, cfg, "not.declared"),
		).rejects.toThrow(/UNKNOWN_PERMISSION/);
	});

	test("throws NOT_A_MEMBER when caller has no role row", async () => {
		const ctx = fakeCtx({ userId: "u_1", tenantId: "o_1" });
		await expect(
			requirePermission(ctx, cfg, "projects.create"),
		).rejects.toThrow(/NOT_A_MEMBER/);
	});

	test("throws FORBIDDEN when role lacks permission", async () => {
		const ctx = fakeCtx({ userId: "u_1", tenantId: "o_1", role: "member" });
		await expect(
			requirePermission(ctx, cfg, "projects.delete"),
		).rejects.toThrow(/FORBIDDEN/);
	});

	test("resolves cleanly when role is permitted", async () => {
		const ctx = fakeCtx({ userId: "u_1", tenantId: "o_1", role: "admin" });
		await expect(
			requirePermission(ctx, cfg, "projects.create"),
		).resolves.toBeUndefined();
	});
});
