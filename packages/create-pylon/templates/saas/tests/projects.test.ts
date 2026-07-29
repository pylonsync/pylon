import { expect, test } from "bun:test";
import { normalizeProjectName } from "../lib/projects";

// Tier 1 (pure logic) — the validation functions/updateProject.ts enforces
// server-side, tested without a running app. Keep decision logic like this in
// lib/ and the handler a thin wrapper, so it's covered here.

test("normalizeProjectName trims and accepts 1–80 chars", () => {
  expect(normalizeProjectName("  Launch  ")).toBe("Launch");
  expect(normalizeProjectName("x")).toBe("x");
  expect(normalizeProjectName("a".repeat(80))).toBe("a".repeat(80));
});

test("normalizeProjectName rejects empty, whitespace-only, and too-long names", () => {
  expect(normalizeProjectName("")).toBeNull();
  expect(normalizeProjectName("   ")).toBeNull();
  expect(normalizeProjectName("a".repeat(81))).toBeNull();
});
