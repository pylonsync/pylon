/**
 * DESIGN.md loader — parses the YAML frontmatter from a DESIGN.md file
 * and emits CSS custom properties so the whole editor reads from a
 * single source of truth. Scope: the editor chrome. The canvas site has
 * its own `Site.tokensJson` that users can edit live (separate layer).
 *
 * The parser handles the DESIGN.md subset we emit (nested keys, scalar
 * leaves, quoted strings, simple arrays). A full YAML implementation is
 * overkill here — if the file schema grows, swap in `js-yaml`.
 */

export type DesignTokens = Record<string, unknown>;

// We bundle the design file via Vite's `?raw` query. This keeps the
// editor self-contained — no extra fetch at runtime.
// @ts-expect-error — Vite-injected raw import
import designMdRaw from "../DESIGN.md?raw";

let cached: DesignTokens | null = null;

export function getDesignTokens(): DesignTokens {
  if (!cached) cached = parseDesignMd(designMdRaw as string);
  return cached;
}

export function parseDesignMd(source: string): DesignTokens {
  const match = source.match(/^---\s*\n([\s\S]*?)\n---/);
  if (!match) return {};
  return parseYaml(match[1]);
}

// Mini YAML parser covering the DESIGN.md vocabulary:
//   key: scalar
//   key:
//     nested: scalar
// Scalars can be quoted "..." or bare. Indentation determines depth
// (2-space steps). Comments after `#` are dropped. That's it.
function parseYaml(src: string): DesignTokens {
  type Frame = { indent: number; container: Record<string, unknown> };
  const root: Record<string, unknown> = {};
  const stack: Frame[] = [{ indent: -1, container: root }];

  const lines = src.split("\n");
  for (const raw of lines) {
    const noComment = raw.replace(/\s+#.*$/, "");
    if (!noComment.trim()) continue;
    const indent = noComment.match(/^ */)?.[0].length ?? 0;
    const line = noComment.slice(indent);
    const colon = line.indexOf(":");
    if (colon < 0) continue;
    const key = stripQuotes(line.slice(0, colon).trim());
    const rest = line.slice(colon + 1).trim();

    // Pop frames that are at >= this indent.
    while (stack.length > 1 && stack[stack.length - 1].indent >= indent) {
      stack.pop();
    }
    const parent = stack[stack.length - 1].container;

    if (rest.length === 0) {
      // Nested block follows.
      const child: Record<string, unknown> = {};
      parent[key] = child;
      stack.push({ indent, container: child });
    } else {
      parent[key] = parseScalar(rest);
    }
  }

  return root;
}

function parseScalar(v: string): unknown {
  const trimmed = v.trim();
  if (trimmed === "true") return true;
  if (trimmed === "false") return false;
  if (/^-?\d+(\.\d+)?$/.test(trimmed)) return Number(trimmed);
  return stripQuotes(trimmed);
}

function stripQuotes(v: string): string {
  if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) {
    return v.slice(1, -1);
  }
  return v;
}

// ---------------------------------------------------------------------------
// Emit CSS custom properties from parsed tokens.
//
// Naming convention:
//   colors.primary          → --color-primary
//   rounded.md              → --rounded-md
//   spacing.4               → --space-4
//   typography.h1.fontSize  → --type-h1-font-size
//   elevation.2             → --elevation-2
//   motion.base             → --motion-base
// ---------------------------------------------------------------------------

export function tokensToCss(tokens: DesignTokens): string {
  const out: string[] = [":root {"];
  const colors = (tokens.colors ?? {}) as Record<string, string>;
  for (const [k, v] of Object.entries(colors)) out.push(`  --color-${k}: ${v};`);

  const rounded = (tokens.rounded ?? {}) as Record<string, string | number>;
  for (const [k, v] of Object.entries(rounded)) out.push(`  --rounded-${k}: ${asDim(v)};`);

  const spacing = (tokens.spacing ?? {}) as Record<string, string | number>;
  for (const [k, v] of Object.entries(spacing)) out.push(`  --space-${k}: ${asDim(v)};`);

  const elev = (tokens.elevation ?? {}) as Record<string, string>;
  for (const [k, v] of Object.entries(elev)) out.push(`  --elevation-${k}: ${v};`);

  const motion = (tokens.motion ?? {}) as Record<string, string | number>;
  for (const [k, v] of Object.entries(motion)) out.push(`  --motion-${k}: ${asDim(v)};`);

  const typo = (tokens.typography ?? {}) as Record<string, Record<string, unknown>>;
  for (const [name, entry] of Object.entries(typo)) {
    if (!entry || typeof entry !== "object") continue;
    for (const [prop, val] of Object.entries(entry)) {
      const cssProp = camelToKebab(prop);
      out.push(`  --type-${name}-${cssProp}: ${String(val)};`);
    }
  }

  out.push("}");
  return out.join("\n");
}

function asDim(v: string | number): string {
  if (typeof v === "number") return `${v}px`;
  // If the string already has a unit we leave it alone. If it looks
  // like a bare integer (`"16"`), append px so the custom property is
  // usable without a `calc(…)` dance.
  return /^-?\d+(\.\d+)?$/.test(v) ? `${v}px` : v;
}

function camelToKebab(s: string): string {
  return s.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
}

// ---------------------------------------------------------------------------
// Install on boot. Idempotent — safe to call multiple times.
// ---------------------------------------------------------------------------

let installed = false;

export function installDesignTokens(): void {
  if (installed) return;
  installed = true;
  const css = tokensToCss(getDesignTokens());
  const style = document.createElement("style");
  style.setAttribute("data-stage-design-tokens", "");
  style.textContent = css;
  document.head.appendChild(style);
}
