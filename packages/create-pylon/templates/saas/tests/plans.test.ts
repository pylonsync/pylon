import { expect, test } from "bun:test";
import {
  FREE_PROJECT_LIMIT,
  annualSavingsPercent,
  annualTotal,
  canCreateProject,
  planById,
  planFromSubscription,
} from "../lib/plans";

// Tier 1 (pure logic) — the cap and the plan derivation that
// functions/createProject.ts and the dashboard rely on.

test("free workspaces stop at the cap, pro never does", () => {
  expect(canCreateProject("free", 0)).toBe(true);
  expect(canCreateProject("free", FREE_PROJECT_LIMIT - 1)).toBe(true);
  expect(canCreateProject("free", FREE_PROJECT_LIMIT)).toBe(false);
  expect(canCreateProject("pro", 10_000)).toBe(true);
});

test("a subscription is pro only while Stripe reports it usable", () => {
  expect(planFromSubscription(null)).toBe("free");
  expect(planFromSubscription({ plan: "pro", status: "active" })).toBe("pro");
  expect(planFromSubscription({ plan: "pro", status: "trialing" })).toBe("pro");
  expect(planFromSubscription({ plan: "pro", status: "past_due" })).toBe("pro");
  expect(planFromSubscription({ plan: "pro", status: "canceled" })).toBe("free");
  expect(planFromSubscription({ plan: "enterprise", status: "active" })).toBe("free");
});

test("annual pricing derives its total and saving from the catalog", () => {
  const pro = planById("pro")!;
  expect(annualTotal(pro)).toBe(pro.annualPerMonth! * 12);
  expect(annualSavingsPercent(pro)).toBeGreaterThan(0);
  expect(annualSavingsPercent(planById("free")!)).toBe(0);
});
