// Page metadata in the served HTML.
//
// Veritas pages exported `metadata` and the HTML had none of it: the server
// put the tags inside the React tree beside the page, and the field layout
// renders a loading screen on the server instead of its children (the
// member check runs in the browser), so the tags were never rendered. The
// server now writes the metadata into <head> itself, merges layout metadata
// under the page's, and replaces the tags a root layout writes in its own
// <head> instead of adding a second set.
//
// ssr-runtime.ts pulls in runtime.ts (the bun runner entrypoint, which runs
// main() on import), so each render runs in a child process, like
// ssr-offline-shell.test.ts.

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { dropReplacedHeadTags, headTagKeys, mergeMetadata } from "./ssr-runtime";

const SSR_RUNTIME = join(import.meta.dir, "ssr-runtime.ts");
const PKG_NODE_MODULES = join(import.meta.dir, "..", "node_modules");

function makeApp(files: Record<string, string>): string {
  const dir = mkdtempSync(join(tmpdir(), "pylon-metadata-"));
  mkdirSync(join(dir, "app"), { recursive: true });
  symlinkSync(PKG_NODE_MODULES, join(dir, "node_modules"), "dir");
  for (const [p, src] of Object.entries(files)) {
    mkdirSync(join(dir, p, ".."), { recursive: true });
    writeFileSync(join(dir, p), src);
  }
  return dir;
}

async function render(
  dir: string,
  layouts: string[],
  headers: Record<string, string> = {},
  component = "app/page",
): Promise<string> {
  const msg = {
    type: "render_route", call_id: "c1", component, layouts,
    route_path: "/", url: "/", host: "app.example", params: {}, search_params: {},
    headers: { host: "app.example", ...headers }, cookies: {}, auth: {}, session_present: false,
  };
  const probe = `
import { handleRenderRoute } from ${JSON.stringify(SSR_RUNTIME)};
const frames = [];
await handleRenderRoute(${JSON.stringify(msg)}, (m) => frames.push(m));
console.error("FRAMES " + JSON.stringify(frames));
setTimeout(() => process.exit(0), 50);
`;
  const probePath = join(dir, "probe.ts");
  writeFileSync(probePath, probe);
  const proc = Bun.spawn(["bun", "run", probePath], { cwd: dir, env: { ...process.env, PYLON_DEV_MODE: "0" }, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
  const [out, err] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  const code = await proc.exited;
  const line = err.split("\n").find((l) => l.includes("FRAMES "));
  if (code !== 0 || !line) throw new Error(`probe failed (exit ${code})\nstdout: ${out}\nstderr: ${err}`);
  const frames = JSON.parse(line.slice(line.indexOf("FRAMES ") + 7)) as any[];
  return frames.filter((f) => f.type === "render_chunk").map((f) => Buffer.from(f.data, "base64").toString("utf8")).join("");
}


// A root layout like Veritas's: its own <head> with default link-preview tags.
const ROOT_LAYOUT = `export default function RootLayout({ children }: any) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <title>Field Sales</title>
        <meta name="description" content="The field app." />
        <meta property="og:title" content="Field Sales" />
        <meta property="og:image" content="https://app.example/og.png" />
        <meta property="og:image:width" content="1200" />
        <meta name="twitter:title" content="Field Sales" />
        <script dangerouslySetInnerHTML={{ __html: 'var t = "<title>kept</title>";' }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
`;

// A layout that decides access in the browser: on the server it renders a
// loading screen, never its children.
const GATE_LAYOUT = `export default function Gate({ children }: any) {
  return typeof window === "undefined" ? <p>Loading</p> : children;
}
`;

const count = (html: string, re: RegExp) => (html.match(re) ?? []).length;
// The document up to </head>, without script text (the root layout's script
// holds a <title> string that is not a tag).
const headOf = (html: string) =>
  html.slice(0, html.indexOf("</head>")).replace(/<script\b[\s\S]*?<\/script>/g, "");

describe("page metadata in the HTML", () => {
  test("reaches <head> when a layout does not render the page, and replaces the root layout's tags", async () => {
    const dir = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/gate/layout.tsx": GATE_LAYOUT,
      "app/page.tsx": `export const metadata = {
  title: "Home · Field Sales",
  openGraph: { title: "Home", description: "Today's doors." },
  twitter: { title: "Home" },
};
export default function P() { return <p>page body</p>; }
`,
    });
    const html = await render(dir, ["app/layout", "app/gate/layout"]);
    expect(html).toContain("Loading");
    const head = headOf(html);
    expect(html).not.toContain("page body");
    expect(count(head, /<title>/g)).toBe(1);
    expect(head).toContain("<title>Home · Field Sales</title>");
    expect(count(head, /property="og:title"/g)).toBe(1);
    expect(head).toContain('property="og:title" content="Home"');
    expect(head).toContain('property="og:description" content="Today&#x27;s doors."');
    expect(count(head, /name="twitter:title"/g)).toBe(1);
    expect(head).toContain('name="twitter:title" content="Home"');
    // Tags the page does not set stay as the layout wrote them.
    expect(head).toContain('name="description" content="The field app."');
    expect(head).toContain('property="og:image" content="https://app.example/og.png"');
    expect(head).toContain('property="og:image:width" content="1200"');
    // Text inside a script is not a tag.
    expect(html).toContain('var t = "<title>kept</title>";');
  });

  test("a page's og:image replaces the layout's image and its dimensions", async () => {
    const dir = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/page.tsx": `export const metadata = { openGraph: { image: "https://app.example/home.png" } };
export default function P() { return <p>x</p>; }
`,
    });
    const head = headOf(await render(dir, ["app/layout"]));
    expect(count(head, /property="og:image"/g)).toBe(1);
    expect(head).toContain('property="og:image" content="https://app.example/home.png"');
    expect(head).not.toContain("og:image:width");
    expect(head).toContain("<title>Field Sales</title>");
  });

  test("a layout's metadata applies to its pages, and the page's values win", async () => {
    const dir = makeApp({
      "app/layout.tsx": `export const metadata = { title: "Site", description: "From the layout.", openGraph: { siteName: "Site" } };
export default function L({ children }: any) { return <html><head><meta charSet="utf-8" /></head><body>{children}</body></html>; }
`,
      "app/inner/layout.tsx": `export async function generateMetadata() { return { robots: "noindex" }; }
export default function L({ children }: any) { return children; }
`,
      "app/page.tsx": `export const metadata = { title: "Page", openGraph: { title: "Page OG" } };
export default function P() { return <p>x</p>; }
`,
    });
    const head = headOf(await render(dir, ["app/layout", "app/inner/layout"]));
    expect(count(head, /<title>/g)).toBe(1);
    expect(head).toContain("<title>Page</title>");
    expect(head).toContain('name="description" content="From the layout."');
    expect(head).toContain('name="robots" content="noindex"');
    expect(head).toContain('property="og:site_name" content="Site"');
    expect(head).toContain('property="og:title" content="Page OG"');
  });

  test("an offline shell request carries the metadata", async () => {
    const dir = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/page.tsx": `export const metadata = { title: "Home · Field Sales" };
export default function P() { return <p>x</p>; }
`,
    });
    const head = headOf(await render(dir, ["app/layout"], { "x-pylon-offline-shell": "1" }));
    expect(count(head, /<title>/g)).toBe(1);
    expect(head).toContain("<title>Home · Field Sales</title>");
  });

  test("a navigation response carries the page's title, or the layout's when the page has none", async () => {
    const titled = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/page.tsx": `export const metadata = { title: "Jobs" };
export default function P() { return <p>x</p>; }
`,
    });
    const nav = await render(titled, ["app/layout"], { "x-pylon-nav": "1" });
    expect(count(nav, /<title>/g)).toBe(1);
    expect(nav).toContain("<title>Jobs</title>");

    const untitled = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/page.tsx": `export default function P() { return <p>x</p>; }`,
    });
    const nav2 = await render(untitled, ["app/layout"], { "x-pylon-nav": "1" });
    expect(count(nav2, /<title>/g)).toBe(1);
    expect(nav2).toContain("<title>Field Sales</title>");
  });

  test("a not-found boundary's metadata replaces the layout's title", async () => {
    const dir = makeApp({
      "app/layout.tsx": ROOT_LAYOUT,
      "app/not-found.tsx": `export const metadata = { title: "Not found" };
export default function NF() { return <p>missing</p>; }
`,
    });
    // The host renders `app/not-found` by name for an unmatched URL.
    const head = headOf(await render(dir, ["app/layout"], {}, "app/not-found"));
    expect(count(head, /<title>/g)).toBe(1);
    expect(head).toContain("<title>Not found</title>");

    // A page that calls notFound() renders the same boundary.
    writeFileSync(
      join(dir, "app", "page.tsx"),
      `export default function P(props: any) { props.response.notFound(); }`,
    );
    const caught = headOf(await render(dir, ["app/layout"]));
    expect(count(caught, /<title>/g)).toBe(1);
    expect(caught).toContain("<title>Not found</title>");
    expect(caught).toContain('property="og:title" content="Field Sales"');
  });
});

describe("dropReplacedHeadTags", () => {
  test("drops only the tags the metadata sets, by title, name, property and canonical", () => {
    const head =
      '<head><title>Old</title><meta name="description" content="a > b"/>' +
      '<meta property="og:image" content="x"/><meta property="og:image:alt" content="y"/>' +
      '<meta name="viewport" content="w"/><link rel="canonical" href="/old"/>' +
      '<link rel="stylesheet" href="/s.css"/><style>title{}</style>';
    const keys = headTagKeys(
      '<title>New</title><meta data-pylon-meta="" name="description" content="n"/>' +
        '<meta data-pylon-meta="" property="og:image" content="z"/><link data-pylon-meta="" rel="canonical" href="/new"/>',
    );
    expect(dropReplacedHeadTags(head, keys)).toBe(
      '<head><meta name="viewport" content="w"/><link rel="stylesheet" href="/s.css"/><style>title{}</style>',
    );
  });
});

describe("mergeMetadata", () => {
  test("merges openGraph field by field; a new image drops the old image's fields", () => {
    expect(
      mergeMetadata(
        { title: "A", openGraph: { siteName: "S", image: "/a.png", imageWidth: 1200 }, twitter: { image: "/a.png", imageAlt: "a" } },
        { title: "B", openGraph: { image: "/b.png" }, twitter: { image: "/b.png" } },
      ),
    ).toEqual({
      title: "B",
      openGraph: { siteName: "S", image: "/b.png" },
      twitter: { image: "/b.png" },
    });
  });
});
