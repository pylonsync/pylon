// JSON-LD is read by machines and repeated to users, so the tests here are
// about truthfulness as much as shape: the graph must link its own nodes, must
// not claim anything the site does not show, and must not be able to break out
// of the script tag it ships in.

import { describe, expect, test } from "bun:test";
import {
  homepageGraph,
  organizationLd,
  organizationPageGraph,
  serializeJsonLd,
  softwareApplicationLd,
} from "./jsonld";

describe("organization", () => {
  const org = organizationLd() as any;

  test("carries both a contactPoint and an address", () => {
    expect(org["@type"]).toBe("Organization");
    expect(org.contactPoint.length).toBeGreaterThan(0);
    for (const cp of org.contactPoint) {
      expect(cp["@type"]).toBe("ContactPoint");
      expect(cp.contactType).toBeTruthy();
      expect(cp.email).toBe("support@pylonsync.com");
    }
    expect(org.address["@type"]).toBe("PostalAddress");
    expect(org.address.addressCountry).toBe("US");
  });

  test("sameAs points only at profiles we control", () => {
    expect(org.sameAs).toContain("https://github.com/pylonsync/pylon");
    for (const url of org.sameAs) expect(url).toStartWith("https://");
  });
});

describe("software application", () => {
  const app = softwareApplicationLd() as any;

  test("is a free developer application with a licence and a repo", () => {
    expect(app["@type"]).toBe("SoftwareApplication");
    expect(app.applicationCategory).toBe("DeveloperApplication");
    expect(app.offers.price).toBe("0");
    expect(app.offers.priceCurrency).toBe("USD");
    expect(app.license).toContain("mit");
    expect(app.codeRepository).toBe("https://github.com/pylonsync/pylon");
  });

  test("claims no rating", () => {
    // A review count nobody can check is the fastest way to lose the rich
    // result and the reader's trust.
    expect(app.aggregateRating).toBeUndefined();
    expect(app.review).toBeUndefined();
  });

  test("is attributed to the organization node, not a copy of it", () => {
    expect(app.publisher).toEqual({ "@id": "https://www.pylonsync.com/#organization" });
  });
});

describe("graphs", () => {
  test("the homepage graph is one connected graph", () => {
    const graph = homepageGraph() as any;
    expect(graph["@context"]).toBe("https://schema.org");
    const ids = new Set(graph["@graph"].map((n: any) => n["@id"]));
    expect(ids.has("https://www.pylonsync.com/#organization")).toBe(true);
    expect(ids.has("https://www.pylonsync.com/#software")).toBe(true);
    expect(ids.has("https://www.pylonsync.com/#website")).toBe(true);
    // Every @id reference resolves to a node in the same graph.
    const refs = JSON.stringify(graph).match(/\{"@id":"[^"]+"\}/g) ?? [];
    for (const ref of refs) {
      const id = JSON.parse(ref)["@id"];
      expect(ids.has(id), `dangling @id ${id}`).toBe(true);
    }
  });

  test("an organization page graph names the page and points at the publisher", () => {
    const graph = organizationPageGraph({
      path: "/about",
      name: "About Pylon",
      description: "What Pylon is.",
    }) as any;
    const page = graph["@graph"].find((n: any) => n["@type"] === "WebPage");
    expect(page.url).toBe("https://www.pylonsync.com/about");
    expect(page.about).toEqual({ "@id": "https://www.pylonsync.com/#organization" });
  });

  test("every url is absolute and on the canonical host", () => {
    // A mixed apex/www graph tells a crawler there are two sites.
    const urls = JSON.stringify(homepageGraph()).match(/https?:\/\/[^"]+/g) ?? [];
    const own = urls.filter((u) => u.includes("pylonsync.com"));
    expect(own.length).toBeGreaterThan(0);
    for (const u of own) {
      expect(u.startsWith("https://www.pylonsync.com") || u.startsWith("https://docs.pylonsync.com")).toBe(
        true,
      );
    }
  });
});

describe("serialization", () => {
  test("escapes < so a value cannot close the script tag", () => {
    const out = serializeJsonLd({ name: "</script><img onerror=alert(1)>" });
    expect(out).not.toContain("</script");
    expect(out).toContain("\\u003c");
    expect(JSON.parse(out).name).toBe("</script><img onerror=alert(1)>");
  });

  test("round-trips a real graph", () => {
    expect(JSON.parse(serializeJsonLd(homepageGraph()))).toEqual(
      JSON.parse(JSON.stringify(homepageGraph())),
    );
  });
});
