// Design-mode JSX stamp: every DOM element under app/, components/ and
// .design/ gets `data-pylon-src="<rel>:<line>:<col>"`. Components are left
// alone, the transform is idempotent, and files outside the scoped dirs are
// never touched. The last case runs the real preload in a child Bun process
// so the plugin wiring (filter + loader) is covered, not only the transform.

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  DESIGN_STAMP_ATTR,
  designStampScopePattern,
  relativeSourcePath,
  stampJsxSource,
} from "./design-stamp";

const STAMP = /data-pylon-src="([^"]+)"/g;

function stamps(src: string): string[] {
  return [...src.matchAll(STAMP)].map((m) => m[1]);
}

describe("stampJsxSource", () => {
  test("stamps DOM elements with 1-based line and column of the opening <", () => {
    const src = [
      "export default function Page() {",
      "  return (",
      "    <main className=\"p-4\">",
      "      <h1>Hello</h1>",
      "      <img src=\"/a.png\" />",
      "    </main>",
      "  );",
      "}",
      "",
    ].join("\n");
    const out = stampJsxSource(src, "app/page.tsx");
    expect(stamps(out)).toEqual([
      "app/page.tsx:3:5",
      "app/page.tsx:4:7",
      "app/page.tsx:5:7",
    ]);
    // Existing attributes survive.
    expect(out).toContain('className="p-4"');
    expect(out).toContain('src="/a.png"');
  });

  test("components and member-expression tags are not stamped", () => {
    const src = [
      "import { Card } from \"../components/card\";",
      "const UI = { Box: (p: any) => <div {...p} /> };",
      "export default function Page() {",
      "  return (",
      "    <Card title=\"x\">",
      "      <UI.Box>",
      "        <span>inner</span>",
      "      </UI.Box>",
      "      <Card.Footer />",
      "    </Card>",
      "  );",
      "}",
      "",
    ].join("\n");
    const out = stampJsxSource(src, "app/page.tsx");
    // Only the <div> in the inline component and the <span> are DOM.
    expect(stamps(out)).toEqual(["app/page.tsx:2:31", "app/page.tsx:7:9"]);
    expect(out).not.toMatch(/<Card[^>]*data-pylon-src/);
    expect(out).not.toMatch(/<UI\.Box[^>]*data-pylon-src/);
    expect(out).not.toMatch(/<Card\.Footer[^>]*data-pylon-src/);
  });

  test("is idempotent and keeps a hand-written attribute", () => {
    const src = [
      "export const A = () => (",
      "  <section data-pylon-src=\"custom:1:1\">",
      "    <p>x</p>",
      "  </section>",
      ");",
      "",
    ].join("\n");
    const once = stampJsxSource(src, "components/a.tsx");
    expect(stamps(once)).toEqual(["custom:1:1", "components/a.tsx:3:5"]);
    const twice = stampJsxSource(once, "components/a.tsx");
    expect(twice).toBe(once);
  });

  test("returns the source unchanged when there is nothing to stamp", () => {
    const src = "export const n = 1;\nexport const C = () => <Comp />;\n";
    expect(stampJsxSource(src, "app/x.tsx")).toBe(src);
  });

  test("handles fragments, conditionals, and .jsx files", () => {
    const src = [
      "export default function P({ ok }) {",
      "  return (",
      "    <>",
      "      {ok ? <b>yes</b> : <i>no</i>}",
      "      {[1, 2].map((n) => <li key={n}>{n}</li>)}",
      "    </>",
      "  );",
      "}",
      "",
    ].join("\n");
    const out = stampJsxSource(src, "app/p.jsx");
    expect(stamps(out)).toEqual(["app/p.jsx:4:13", "app/p.jsx:4:26", "app/p.jsx:5:26"]);
    expect(out).toContain("key={n}");
  });
});

describe("scope", () => {
  test("pattern matches only tsx/jsx under app, components and .design", () => {
    const root = "/proj/my app";
    const re = designStampScopePattern(root);
    expect(re.test("/proj/my app/app/page.tsx")).toBe(true);
    expect(re.test("/proj/my app/app/blog/[slug]/page.tsx")).toBe(true);
    expect(re.test("/proj/my app/components/card.jsx")).toBe(true);
    expect(re.test("/proj/my app/.design/variants/v1.tsx")).toBe(true);
    expect(re.test("/proj/my app/lib/ui.tsx")).toBe(false);
    expect(re.test("/proj/my app/app/data.ts")).toBe(false);
    expect(re.test("/proj/my app/node_modules/x/app/page.tsx")).toBe(false);
    expect(re.test("/proj/my apple/app/page.tsx")).toBe(false);
  });

  test("relativeSourcePath strips the root", () => {
    expect(relativeSourcePath("/proj", "/proj/app/page.tsx")).toBe("app/page.tsx");
    expect(relativeSourcePath("/proj/", "/proj/.design/v.tsx")).toBe(".design/v.tsx");
  });
});

// The preload itself: a child Bun process with PYLON_DESIGN_MODE=1 and
// `--preload design-stamp.ts` renders two modules to static markup. The one
// under app/ is stamped; the one under lib/ is not.
test("preload stamps app/ modules and leaves lib/ modules alone", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-design-stamp-"));
  mkdirSync(join(dir, "app"), { recursive: true });
  mkdirSync(join(dir, "lib"), { recursive: true });
  const body =
    "export default function C() { return <div className=\"c\"><span>t</span></div>; }\n";
  writeFileSync(join(dir, "app", "page.tsx"), body);
  writeFileSync(join(dir, "lib", "widget.tsx"), body);
  // Resolve react from this package's node_modules (the temp dir has none).
  const reactDir = join(import.meta.dir, "..");
  const probe = `
import React from ${JSON.stringify(join(reactDir, "node_modules/react"))};
import { renderToStaticMarkup } from ${JSON.stringify(join(reactDir, "node_modules/react-dom/server"))};
const a = (await import(${JSON.stringify(join(dir, "app/page.tsx"))})).default;
const b = (await import(${JSON.stringify(join(dir, "lib/widget.tsx"))})).default;
console.log("APP " + renderToStaticMarkup(React.createElement(a)));
console.log("LIB " + renderToStaticMarkup(React.createElement(b)));
`;
  writeFileSync(join(dir, "probe.ts"), probe);
  const preload = join(import.meta.dir, "design-stamp.ts");
  const proc = Bun.spawn(["bun", "run", "--preload", preload, join(dir, "probe.ts")], {
    cwd: dir,
    env: { ...process.env, PYLON_DESIGN_MODE: "1" },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  expect(await proc.exited).toBe(0);
  if (err.trim()) console.error(err);
  const app = out.split("\n").find((l) => l.startsWith("APP "))!;
  const lib = out.split("\n").find((l) => l.startsWith("LIB "))!;
  expect(app).toContain(`${DESIGN_STAMP_ATTR}="app/page.tsx:1:38"`);
  expect(app).toContain(`${DESIGN_STAMP_ATTR}="app/page.tsx:1:57"`);
  expect(lib).not.toContain(DESIGN_STAMP_ATTR);
});
