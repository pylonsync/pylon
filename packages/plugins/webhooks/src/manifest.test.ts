import { describe, expect, test } from "bun:test";

import { buildWebhookManifest } from "./manifest";

describe("webhook manifest credentials", () => {
	test("always keeps signing secrets and custom headers server-only", () => {
		const endpoint = buildWebhookManifest({}).entities[0];

		expect(endpoint.fields.secret._def.serverOnly).toBe(true);
		expect(endpoint.fields.headers._def.serverOnly).toBe(true);
		expect(endpoint.fields.secret._def.encrypted).toBeUndefined();
		expect(endpoint.fields.headers._def.encrypted).toBeUndefined();
	});

	test("supports explicit at-rest encryption when the deployment has a key", () => {
		const endpoint = buildWebhookManifest({
			encryptCredentials: true,
		}).entities[0];

		expect(endpoint.fields.secret._def.serverOnly).toBe(true);
		expect(endpoint.fields.headers._def.serverOnly).toBe(true);
		expect(endpoint.fields.secret._def.encrypted).toBe(true);
		expect(endpoint.fields.headers._def.encrypted).toBe(true);
	});
});
