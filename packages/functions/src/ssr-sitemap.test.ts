// Tests for the app/sitemap.ts → /sitemap.xml and app/robots.ts → /robots.txt
// data-route conventions: the pure serializers (where escaping / shape bugs
// live) plus handleDataRoute end-to-end (import a temp module, serialize, emit
// the response_start/chunk/done protocol with the right content-type).

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  handleDataRoute,
  serializeRobots,
  serializeSitemap,
  type RenderRouteMessage,
} from "./ssr-runtime";

describe("serializeSitemap", () => {
  test("empty / undefined → valid empty urlset", () => {
    for (const v of [[], undefined as any]) {
      const xml = serializeSitemap(v);
      expect(xml).toContain('<?xml version="1.0" encoding="UTF-8"?>');
      expect(xml).toContain(
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">',
      );
      expect(xml).toContain("</urlset>");
    }
  });

  test("emits loc + optional fields in order", () => {
    const xml = serializeSitemap([
      {
        url: "https://x.com/blog",
        lastModified: "2026-01-02",
        changeFrequency: "weekly",
        priority: 0.8,
      },
    ]);
    expect(xml).toContain(
      "<url><loc>https://x.com/blog</loc><lastmod>2026-01-02</lastmod><changefreq>weekly</changefreq><priority>0.8</priority></url>",
    );
  });

  test("Date lastModified is ISO-serialized", () => {
    const d = new Date("2026-06-12T10:00:00.000Z");
    const xml = serializeSitemap([{ url: "https://x.com/", lastModified: d }]);
    expect(xml).toContain("<lastmod>2026-06-12T10:00:00.000Z</lastmod>");
  });

  test("XML-escapes loc and values", () => {
    const xml = serializeSitemap([
      { url: "https://x.com/s?a=1&b=2<c>'\"" },
    ]);
    expect(xml).toContain(
      "<loc>https://x.com/s?a=1&amp;b=2&lt;c&gt;&apos;&quot;</loc>",
    );
    // raw unescaped specials must not leak into the document body
    expect(xml.includes("a=1&b=2")).toBe(false);
  });

  test("hreflang alternates add the xhtml namespace + link tags", () => {
    const xml = serializeSitemap([
      {
        url: "https://x.com/en",
        alternates: {
          languages: { "en-US": "https://x.com/en", "es-ES": "https://x.com/es" },
        },
      },
    ]);
    expect(xml).toContain('xmlns:xhtml="http://www.w3.org/1999/xhtml"');
    expect(xml).toContain(
      '<xhtml:link rel="alternate" hreflang="en-US" href="https://x.com/en"/>',
    );
    expect(xml).toContain('hreflang="es-ES"');
  });

  test("no xhtml namespace when no alternates present", () => {
    const xml = serializeSitemap([{ url: "https://x.com/" }]);
    expect(xml.includes("xmlns:xhtml")).toBe(false);
  });

  test("skips entries without a string url", () => {
    const xml = serializeSitemap([
      { url: "https://x.com/ok" },
      { url: undefined as any },
      null as any,
    ]);
    expect((xml.match(/<url>/g) || []).length).toBe(1);
  });
});

describe("serializeRobots", () => {
  test("single rule with default user-agent", () => {
    const txt = serializeRobots({ rules: { allow: "/", disallow: "/admin" } });
    expect(txt).toContain("User-agent: *");
    expect(txt).toContain("Allow: /");
    expect(txt).toContain("Disallow: /admin");
    expect(txt.endsWith("\n")).toBe(true);
  });

  test("array userAgent + array allow/disallow + crawlDelay", () => {
    const txt = serializeRobots({
      rules: {
        userAgent: ["Googlebot", "Bingbot"],
        disallow: ["/a", "/b"],
        crawlDelay: 5,
      },
    });
    expect(txt).toContain("User-agent: Googlebot");
    expect(txt).toContain("User-agent: Bingbot");
    expect(txt).toContain("Disallow: /a");
    expect(txt).toContain("Disallow: /b");
    expect(txt).toContain("Crawl-delay: 5");
  });

  test("multiple rules, sitemap (array) + host", () => {
    const txt = serializeRobots({
      rules: [
        { userAgent: "*", disallow: "/private" },
        { userAgent: "BadBot", disallow: "/" },
      ],
      sitemap: ["https://x.com/sitemap.xml", "https://x.com/news.xml"],
      host: "https://x.com",
    });
    expect(txt).toContain("User-agent: BadBot");
    expect(txt).toContain("Sitemap: https://x.com/sitemap.xml");
    expect(txt).toContain("Sitemap: https://x.com/news.xml");
    expect(txt).toContain("Host: https://x.com");
  });

  test("undefined robots → just a trailing newline", () => {
    expect(serializeRobots(undefined)).toBe("\n");
  });
});

describe("handleDataRoute (end-to-end import + serialize)", () => {
  const tmpdirs: string[] = [];
  const prevCwd = process.cwd();
  afterEach(() => {
    process.chdir(prevCwd);
    for (const d of tmpdirs.splice(0)) {
      try {
        fs.rmSync(d, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    }
  });

  function fixture(file: string, src: string): void {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-data-route-"));
    tmpdirs.push(dir);
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", file), src);
    process.chdir(dir);
  }

  const collect = async (
    component: string,
    kind: "sitemap" | "robots",
  ): Promise<any[]> => {
    const sent: any[] = [];
    const msg = { component, call_id: "c1" } as unknown as RenderRouteMessage;
    await handleDataRoute(msg, kind, (m) => sent.push(m));
    return sent;
  };

  test("async app/sitemap.ts → 200 application/xml with the rendered urls", async () => {
    fixture(
      "sitemap.ts",
      `export default async function sitemap() {
        return [{ url: "https://x.com/", priority: 1.0, changeFrequency: "daily" }];
      }`,
    );
    const sent = await collect("app/sitemap", "sitemap");
    const start = sent.find((m) => m.type === "response_start");
    const chunk = sent.find((m) => m.type === "render_chunk");
    expect(start.status).toBe(200);
    expect(start.headers["content-type"]).toContain("application/xml");
    expect(start.headers["cache-control"]).toBe("public, max-age=3600");
    const body = Buffer.from(chunk.data, "base64").toString("utf8");
    expect(body).toContain("<loc>https://x.com/</loc>");
    expect(body).toContain("<priority>1</priority>");
    expect(sent.some((m) => m.type === "render_done")).toBe(true);
  });

  test("app/robots.ts → 200 text/plain", async () => {
    fixture(
      "robots.ts",
      `export default function robots() {
        return { rules: { userAgent: "*", allow: "/", disallow: "/dashboard" }, sitemap: "https://x.com/sitemap.xml" };
      }`,
    );
    const sent = await collect("app/robots", "robots");
    const start = sent.find((m) => m.type === "response_start");
    const body = Buffer.from(
      sent.find((m) => m.type === "render_chunk").data,
      "base64",
    ).toString("utf8");
    expect(start.headers["content-type"]).toContain("text/plain");
    expect(body).toContain("Disallow: /dashboard");
    expect(body).toContain("Sitemap: https://x.com/sitemap.xml");
  });

  test("a throwing sitemap surfaces as a 500 (does not wedge the runner)", async () => {
    fixture(
      "sitemap.ts",
      `export default function sitemap() { throw new Error("boom"); }`,
    );
    const sent = await collect("app/sitemap", "sitemap");
    const start = sent.find((m) => m.type === "response_start");
    expect(start.status).toBe(500);
    const body = Buffer.from(
      sent.find((m) => m.type === "render_chunk").data,
      "base64",
    ).toString("utf8");
    expect(body).toContain("boom");
    expect(sent.some((m) => m.type === "render_done")).toBe(true);
  });
});
