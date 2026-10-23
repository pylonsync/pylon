// Per-route vendor code for "barrel" packages.
//
// A barrel is a package whose entry only re-exports other packages as
// namespaces, like `radix-ui`:
//
//   import * as Dialog from "@radix-ui/react-dialog";
//   import * as Select from "@radix-ui/react-select";
//   export { Dialog, Select };
//
// With code splitting, every route that imports anything from the barrel
// reaches every package it re-exports, so the bundler puts all of them in
// one shared chunk that loads on every such route (55 KB gzip of Radix on
// the Veritas home screen, which uses one Radix component). Rewriting
//
//   import { Dialog as SheetPrimitive } from "radix-ui";
//
// in app source to
//
//   import * as SheetPrimitive from "@radix-ui/react-dialog";
//
// lets each route reach only the packages it uses, so the splitter keeps
// the rest out of its chunks. The client bundler applies this to app files
// (not node_modules) for every bare import whose entry is a barrel.

/**
 * The namespaces a barrel entry re-exports (exported name → module
 * specifier), or null when `source` is not a pure namespace barrel: any
 * statement other than `import * as X from "..."` and `export { ... }` of
 * those names makes it null.
 */
export function parseNamespaceBarrel(source: string): Map<string, string> | null {
  const code = source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "")
    .replace(/["']use client["'];?/g, "");
  const imports = new Map<string, string>();
  let rest = code.replace(
    /import\s+\*\s+as\s+([A-Za-z_$][\w$]*)\s+from\s+(["'])([^"']+)\2\s*;?/g,
    (_m, local: string, _q: string, spec: string) => {
      imports.set(local, spec);
      return "";
    },
  );
  const exported = new Map<string, string>();
  let ok = true;
  rest = rest.replace(/export\s*\{([^}]*)\}\s*;?/g, (_m, list: string) => {
    for (const item of list.split(",")) {
      const part = item.trim();
      if (!part) continue;
      const m = part.match(/^([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?$/);
      const spec = m ? imports.get(m[1]!) : undefined;
      if (!m || !spec) {
        ok = false;
        continue;
      }
      exported.set(m[2] ?? m[1]!, spec);
    }
    return "";
  });
  if (!ok || rest.trim() !== "" || exported.size === 0) return null;
  return exported;
}

/**
 * Rewrite `import { A, B as C } from "<barrel>"` in `source` to one
 * namespace import per name, from `resolveTarget(spec)` (the specifier the
 * importing file should use for the re-exported module). Type-only imports
 * and names the barrel does not re-export stay on the barrel import.
 * `barrels` maps a bare specifier to its parsed barrel. Returns the source
 * unchanged when nothing applies.
 */
export function rewriteBarrelImports(
  source: string,
  barrels: (specifier: string) => Map<string, string> | null,
  resolveTarget: (barrel: string, spec: string) => string,
): string {
  return source.replace(
    /import\s+(type\s+)?\{([^}]*)\}\s*from\s*(["'])([^"'./][^"']*)\3\s*;?/g,
    (whole, typeOnly: string | undefined, list: string, quote: string, specifier: string) => {
      if (typeOnly) return whole;
      const map = barrels(specifier);
      if (!map) return whole;
      const kept: string[] = [];
      const lines: string[] = [];
      for (const item of list.split(",")) {
        const part = item.trim();
        if (!part) continue;
        const m = part.match(/^(type\s+)?([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?$/);
        const target = m && !m[1] ? map.get(m[2]!) : undefined;
        if (!m || !target) {
          kept.push(part);
          continue;
        }
        const local = m[3] ?? m[2]!;
        lines.push(`import * as ${local} from ${quote}${resolveTarget(specifier, target)}${quote};`);
      }
      if (lines.length === 0) return whole;
      if (kept.length > 0) {
        lines.unshift(`import { ${kept.join(", ")} } from ${quote}${specifier}${quote};`);
      }
      return lines.join(" ");
    },
  );
}
