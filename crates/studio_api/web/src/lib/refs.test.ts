import { describe, expect, test } from "bun:test";
import type { Manifest, ManifestEntity } from "./pylon";
import { displayField, refTarget, refTargetOfType, rowLabel } from "./refs";

const company: ManifestEntity = {
	name: "Company",
	fields: [
		{ name: "name", type: "string" },
		{ name: "domain", type: "string", optional: true },
	],
};
const deal: ManifestEntity = {
	name: "Deal",
	fields: [
		{ name: "title", type: "string" },
		{ name: "companyId", type: "id(Company)", optional: true },
		{ name: "owner", type: "id(User)" },
		{ name: "contact_id", type: "string" },
		{ name: "valid", type: "bool" },
	],
};
const contact: ManifestEntity = {
	name: "Contact",
	fields: [{ name: "email", type: "string" }],
};
const manifest: Manifest = {
	manifest_version: 1,
	name: "test",
	version: "0.0.0",
	entities: [company, deal, contact],
	queries: [],
	actions: [],
	policies: [],
	routes: [],
};

describe("refTargetOfType", () => {
	test("reads the entity from an id type", () => {
		expect(refTargetOfType("id(Company)")).toBe("Company");
		expect(refTargetOfType("id( Org_Member )")).toBe("Org_Member");
	});

	test("returns null for other types", () => {
		expect(refTargetOfType("string")).toBeNull();
		expect(refTargetOfType("id")).toBeNull();
		expect(refTargetOfType("id()")).toBeNull();
	});
});

describe("refTarget", () => {
	test("uses the manifest type", () => {
		expect(refTarget(manifest, deal, "companyId")).toBe("Company");
	});

	test("ignores a typed reference to an entity that is not in the manifest", () => {
		expect(refTarget(manifest, deal, "owner")).toBeNull();
	});

	test("falls back to the <entity>Id name when the type is not a reference", () => {
		expect(refTarget(manifest, deal, "contact_id")).toBe("Contact");
		expect(refTarget(manifest, undefined, "companyId")).toBe("Company");
	});

	test("does not treat other fields as references", () => {
		expect(refTarget(manifest, deal, "title")).toBeNull();
		expect(refTarget(manifest, deal, "valid")).toBeNull();
		expect(refTarget(manifest, deal, "id")).toBeNull();
	});
});

describe("displayField", () => {
	test("prefers a well-known name", () => {
		expect(displayField(company)).toBe("name");
		expect(displayField(deal)).toBe("title");
		expect(displayField(contact)).toBe("email");
	});

	test("falls back to the first required string field", () => {
		expect(
			displayField({
				name: "Note",
				fields: [
					{ name: "summary", type: "string", optional: true },
					{ name: "body", type: "string" },
				],
			}),
		).toBe("body");
	});

	test("returns null when the entity has no string field", () => {
		expect(
			displayField({ name: "Tick", fields: [{ name: "n", type: "int" }] }),
		).toBeNull();
	});
});

describe("rowLabel", () => {
	test("returns the trimmed display value", () => {
		expect(rowLabel(company, { id: "c1", name: " Acme " })).toBe("Acme");
	});

	test("returns null for an empty or missing value", () => {
		expect(rowLabel(company, { id: "c1", name: "" })).toBeNull();
		expect(rowLabel(company, { id: "c1" })).toBeNull();
	});
});
