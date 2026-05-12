import { describe, expect, test } from "bun:test";

import { sendEmail } from "./send";
import type { EmailConfig, EmailCtx } from "./types";

function fakeCtx(env: Record<string, string> = {}): EmailCtx {
	return {
		env,
		auth: {},
		runQuery: async () => undefined as unknown as never,
		runMutation: async () => undefined as unknown as never,
		error: (code, msg) => Object.assign(new Error(`${code}: ${msg}`), { code }),
	};
}

describe("sendEmail", () => {
	test("log provider returns a synthetic message id", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: {
				welcome: {
					subject: "Welcome",
					text: "hi",
				},
			},
		};
		const result = await sendEmail(fakeCtx(), cfg, {
			template: "welcome",
			to: "user@example.com",
		});
		expect(result.messageId).toMatch(/^log_/);
	});

	test("unknown template throws UNKNOWN_TEMPLATE", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: {},
		};
		await expect(
			sendEmail(fakeCtx(), cfg, {
				template: "nope",
				to: "user@example.com",
			}),
		).rejects.toThrow(/UNKNOWN_TEMPLATE/);
	});

	test("suppression filters recipients", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: {
				welcome: { subject: "Hi", text: "hi" },
			},
			isSuppressed: async (_ctx, { email }) => email === "bad@example.com",
		};
		const result = await sendEmail(fakeCtx(), cfg, {
			template: "welcome",
			to: ["bad@example.com", "good@example.com"],
		});
		// Survivor: good@example.com; send succeeds.
		expect(result.suppressed).toBeUndefined();
		expect(result.messageId).toMatch(/^log_/);
	});

	test("all recipients suppressed → returns suppressed flag", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: { welcome: { subject: "Hi", text: "hi" } },
			isSuppressed: async () => true,
		};
		const result = await sendEmail(fakeCtx(), cfg, {
			template: "welcome",
			to: "bounced@example.com",
		});
		expect(result.suppressed).toBe(true);
		expect(result.messageId).toBe("");
	});

	test("function templates render with vars", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: {
				welcome: {
					subject: (v) => `Hi ${v.name}`,
					text: (v) => `Hello ${v.name}`,
				},
			},
		};
		const result = await sendEmail(fakeCtx(), cfg, {
			template: "welcome",
			to: "u@example.com",
			vars: { name: "Eric" },
		});
		expect(result.messageId).toMatch(/^log_/);
	});

	test("missing API key throws MISSING_ENV for non-log providers", async () => {
		const cfg: EmailConfig = {
			provider: "resend",
			from: "test@example.com",
			templates: { welcome: { subject: "Hi", text: "hi" } },
		};
		await expect(
			sendEmail(fakeCtx(), cfg, {
				template: "welcome",
				to: "u@example.com",
			}),
		).rejects.toThrow(/MISSING_ENV/);
	});

	test("template with neither html nor text throws", async () => {
		const cfg: EmailConfig = {
			provider: "log",
			from: "test@example.com",
			templates: { welcome: { subject: "Hi" } },
		};
		await expect(
			sendEmail(fakeCtx(), cfg, {
				template: "welcome",
				to: "u@example.com",
			}),
		).rejects.toThrow(/neither html, text, nor render/);
	});
});
