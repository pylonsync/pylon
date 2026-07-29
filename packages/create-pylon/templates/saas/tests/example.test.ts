import { expect, test } from "bun:test";
import { PRODUCTS, productBySlug } from "../lib/site.config";

// Your first test. Run the suite with:  pylon test  (or: npm test)
//
// `pylon test` discovers every *.test.ts file under tests/ (or functions/) and
// runs it with Bun's test runner against an in-memory Pylon. This example tests
// pure app logic, so it needs no server — a good shape for helpers, validators,
// and data transforms.
//
// To test functions that read or write the database, run `pylon dev` in another
// terminal and call the API over HTTP — see AGENTS.md → Testing for the
// resetDb() isolation pattern.

test("productBySlug finds a product by its slug", () => {
  expect(PRODUCTS.length).toBeGreaterThan(0);
  const sample = PRODUCTS[0];
  expect(productBySlug(sample.slug)).toEqual(sample);
});

test("productBySlug returns undefined for an unknown slug", () => {
  expect(productBySlug("definitely-not-a-real-product")).toBeUndefined();
});
