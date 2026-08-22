// Tests for the app/llms.ts → /llms.txt data-route convention: the pure
// serializer (where the llmstxt.org element order and the newline-escaping
// bugs live) plus handleDataRoute end-to-end.
//
// The format is parsed by tools, not just read by models, so the tests assert
// on exact document structure rather than on "contains the word".

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  handleDataRoute,
  serializeLlms,
  type RenderRouteMessage,
} from "./ssr-runtime";

describe("serializeLlms", () => {
  test("emits the spec's element order", () => {
    const txt = serializeLlms({
      title: "Acme",
      summary: "Invoicing for freelancers.",
      details: ["Use Acme to issue an invoice.", "Free tier, no card."],
      sections: [
        {
          title: "Docs",
          links: [
            { title: "API", url: "https://acme.com/api", notes: "REST + webhooks" },
            { title: "CLI", url: "https://acme.com/cli" },
          ],
        },
        { title: "Optional", links: [{ title: "Blog", url: "https://acme.com/blog" }] },
      ],
    });
    expect(txt).toBe(
      [
        "# Acme",
        "",
        "> Invoicing for freelancers.",
        "",
        "Use Acme to issue an invoice.",
        "",
        "Free tier, no card.",
        "",
        "## Docs",
        "",
        "- [API](https://acme.com/api): REST + webhooks",
        "- [CLI](https://acme.com/cli)",
        "",
        "## Optional",
        "",
        "- [Blog](https://acme.com/blog)",
        "",
      ].join("\n"),
    );
  });

  test("title alone is a valid document", () => {
    expect(serializeLlms({ title: "Acme" })).toBe("# Acme\n");
  });

  test("no title → empty, rather than a headless document", () => {
    expect(serializeLlms(undefined)).toBe("");
    expect(serializeLlms({ title: "   " } as any)).toBe("");
  });

  test("newlines inside a summary or link cannot break the structure", () => {
    // A multi-line summary would end the blockquote after the first line and
    // silently turn the rest into prose.
    const txt = serializeLlms({
      title: "Acme",
      summary: "Line one.\nLine two.",
      sections: [
        {
          title: "Docs",
          links: [{ title: "A\nB", url: "https://acme.com/a", notes: "x\ny" }],
        },
      ],
    });
    expect(txt).toContain("> Line one. Line two.");
    expect(txt).toContain("- [A B](https://acme.com/a): x y");
    expect(txt.split("\n").filter((l) => l.startsWith(">")).length).toBe(1);
  });

  test("headings in the details block are stripped", () => {
    // The first H2 marks where link sections begin; a heading in the prose
    // would swallow everything after it.
    const txt = serializeLlms({
      title: "Acme",
      details: "## Sneaky\nreal prose",
    });
    expect(txt).not.toContain("## Sneaky");
    expect(txt).toContain("Sneaky");
    expect(txt).toContain("real prose");
  });

  test("incomplete links and sections are dropped, not half-emitted", () => {
    const txt = serializeLlms({
      title: "Acme",
      sections: [
        { title: "Docs", links: [{ title: "", url: "https://a" } as any, { title: "B", url: "" } as any] },
        { title: "", links: [{ title: "C", url: "https://c" }] } as any,
      ],
    });
    expect(txt).toContain("## Docs");
    expect(txt).not.toContain("](");
    expect(txt).not.toContain("https://c");
  });
});

describe("handleDataRoute — llms", () => {
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
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-llms-route-"));
    tmpdirs.push(dir);
    fs.mkdirSync(path.join(dir, "app"), { recursive: true });
    fs.writeFileSync(path.join(dir, "app", file), src);
    process.chdir(dir);
  }

  const collect = async (component: string): Promise<any[]> => {
    const sent: any[] = [];
    const msg = { component, call_id: "c1" } as unknown as RenderRouteMessage;
    await handleDataRoute(msg, "llms", (m) => sent.push(m));
    return sent;
  };

  test("async app/llms.ts → 200 text/plain with the rendered document", async () => {
    fixture(
      "llms.ts",
      `export default async function llms() {
        return {
          title: "Acme",
          summary: "Invoicing.",
          sections: [{ title: "Docs", links: [{ title: "API", url: "https://acme.com/api" }] }],
        };
      }`,
    );
    const sent = await collect("app/llms");
    const start = sent.find((m) => m.type === "response_start");
    expect(start.status).toBe(200);
    expect(start.headers["content-type"]).toContain("text/plain");
    expect(start.headers["cache-control"]).toBe("public, max-age=3600");
    const body = Buffer.from(
      sent.find((m) => m.type === "render_chunk").data,
      "base64",
    ).toString("utf8");
    expect(body.startsWith("# Acme\n")).toBe(true);
    expect(body).toContain("- [API](https://acme.com/api)");
    expect(sent.some((m) => m.type === "render_done")).toBe(true);
  });

  test("a throwing llms.ts surfaces as a 500 (does not wedge the runner)", async () => {
    fixture("llms.ts", `export default function llms() { throw new Error("boom"); }`);
    const sent = await collect("app/llms");
    expect(sent.find((m) => m.type === "response_start").status).toBe(500);
    const body = Buffer.from(
      sent.find((m) => m.type === "render_chunk").data,
      "base64",
    ).toString("utf8");
    expect(body).toContain("boom");
  });
});
