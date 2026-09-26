import { describe, expect, test } from "bun:test";
import { monogram } from "../lib/directory";
import { DEMO_SUBMISSIONS, demoDataEnabled } from "../lib/demo";
import { siteConfig } from "../lib/site.config";

describe("monogram", () => {
  test("two words give two initials; one word gives its first two letters", () => {
    expect(monogram("Tidepool DB").letters).toBe("TD");
    expect(monogram("Keyloft").letters).toBe("Ke");
  });
  test("the tint is stable for a name", () => {
    expect(monogram("Keyloft")).toEqual(monogram("Keyloft"));
    expect(monogram("  Keyloft ")).toEqual(monogram("Keyloft"));
  });
});

describe("starter content", () => {
  test("the page headline is not the brand name the nav already shows", () => {
    expect(siteConfig.intro.headline).not.toBe(siteConfig.brand.name);
  });
  test("starter tools avoid names of well-known real products", () => {
    const real = ["Loom", "Inbox Zero", "Quill", "Shipyard", "Tigris", "Cadence", "Watchtower", "Pulse", "Frame", "Schema"];
    for (const l of siteConfig.seedListings) {
      for (const r of real) expect(l.name.toLowerCase()).not.toContain(r.toLowerCase());
    }
  });
});

describe("demo queue", () => {
  test("only under `pylon dev` unless PYLON_DEMO_DATA says otherwise", () => {
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app" })).toBe(true);
    // The Docker base image turns dev mode on by default; without `pylon dev` it must not seed.
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true" })).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "false" })).toBe(false);
    expect(demoDataEnabled({})).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app", PYLON_DEMO_DATA: "no" })).toBe(false);
  });
  test("approved demo submissions match live starter listings; categories are valid", () => {
    const names = new Set(siteConfig.seedListings.map((l) => l.name));
    for (const s of DEMO_SUBMISSIONS) {
      if (s.status === "approved") expect(names.has(s.name)).toBe(true);
      expect(siteConfig.categories).toContain(s.category);
    }
    expect(DEMO_SUBMISSIONS.filter((s) => s.status === "new").length).toBeGreaterThan(3);
  });
});
