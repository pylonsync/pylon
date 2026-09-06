/**
 * Design-mode JSX source stamp.
 *
 * A Bun preload plugin. When `PYLON_DESIGN_MODE=1`, every `.tsx` / `.jsx`
 * module under the app's `app/`, `components/`, and `.design/` directories is
 * rewritten on load so each DOM element carries
 * `data-pylon-src="<relative path>:<line>:<col>"`. The design canvas reads the
 * attribute off the rendered HTML to map a clicked element back to its source.
 *
 * The rewrite runs on the SSR runner only. The client bundler does not load
 * this plugin, so browser bundles are unchanged.
 *
 * The transform uses the TypeScript compiler API. Components (tags that start
 * with an uppercase letter, or member expressions) are left alone; their own
 * DOM elements are stamped where they are written. An element that already has
 * the attribute keeps it, so the transform is idempotent.
 *
 * Load with `bun run --preload <this file> ...`. The plugin registers itself
 * on import; `stampJsxSource` and `designStampScopePattern` are exported for
 * tests and for the runtime that wires the preload.
 */
import ts from "typescript";

export const DESIGN_STAMP_ATTR = "data-pylon-src";

/** True when the design stamp should be active for this process. */
export function isDesignMode(env: Record<string, string | undefined> = process.env): boolean {
  const v = env.PYLON_DESIGN_MODE;
  return v === "1" || v?.toLowerCase() === "true";
}

/** Directories (relative to the project root) whose JSX gets stamped. */
export const DESIGN_STAMP_DIRS = ["app", "components", ".design"] as const;

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Regex matching absolute paths of the `.tsx` / `.jsx` files under the scoped
 * directories of `root`. Used as the `onLoad` filter so files outside the
 * scope never reach the transform.
 */
export function designStampScopePattern(root: string): RegExp {
  const base = escapeRegExp(root.replace(/[\\/]+$/, ""));
  const dirs = DESIGN_STAMP_DIRS.map(escapeRegExp).join("|");
  return new RegExp(`^${base}[\\\\/](?:${dirs})[\\\\/].*\\.(?:tsx|jsx)$`);
}

/** Project-relative path with forward slashes, for the attribute value. */
export function relativeSourcePath(root: string, absPath: string): string {
  const base = root.replace(/[\\/]+$/, "");
  let rel = absPath.startsWith(base) ? absPath.slice(base.length) : absPath;
  rel = rel.replace(/^[\\/]+/, "").replace(/\\/g, "/");
  return rel;
}

function isDomTag(tagName: ts.JsxTagNameExpression): boolean {
  return ts.isIdentifier(tagName) && /^[a-z]/.test(tagName.text);
}

function hasStampAttr(attrs: ts.JsxAttributes): boolean {
  return attrs.properties.some(
    (p) => ts.isJsxAttribute(p) && ts.isIdentifier(p.name) && p.name.text === DESIGN_STAMP_ATTR,
  );
}

/**
 * Add `data-pylon-src="<relPath>:<line>:<col>"` to every DOM element in a
 * JSX/TSX source. `line` and `col` are 1-based and point at the opening `<`.
 * Returns the source unchanged when no element needs a stamp.
 */
export function stampJsxSource(source: string, relPath: string): string {
  const kind = relPath.endsWith(".jsx") ? ts.ScriptKind.JSX : ts.ScriptKind.TSX;
  const sourceFile = ts.createSourceFile(relPath, source, ts.ScriptTarget.Latest, true, kind);
  let stamped = 0;

  const transformer: ts.TransformerFactory<ts.SourceFile> = (ctx) => {
    const f = ctx.factory;
    const stampFor = (node: ts.Node): ts.JsxAttribute => {
      const { line, character } = sourceFile.getLineAndCharacterOfPosition(
        node.getStart(sourceFile),
      );
      const value = `${relPath}:${line + 1}:${character + 1}`;
      return f.createJsxAttribute(
        f.createIdentifier(DESIGN_STAMP_ATTR),
        f.createStringLiteral(value),
      );
    };
    const withStamp = (attrs: ts.JsxAttributes, attr: ts.JsxAttribute): ts.JsxAttributes =>
      f.updateJsxAttributes(attrs, [...attrs.properties, attr]);

    const visit: ts.Visitor = (node) => {
      if (ts.isJsxOpeningElement(node)) {
        const shouldStamp = isDomTag(node.tagName) && !hasStampAttr(node.attributes);
        const attr = shouldStamp ? stampFor(node) : null;
        const visited = ts.visitEachChild(node, visit, ctx);
        if (!attr) return visited;
        stamped++;
        return f.updateJsxOpeningElement(
          visited,
          visited.tagName,
          visited.typeArguments,
          withStamp(visited.attributes, attr),
        );
      }
      if (ts.isJsxSelfClosingElement(node)) {
        const shouldStamp = isDomTag(node.tagName) && !hasStampAttr(node.attributes);
        const attr = shouldStamp ? stampFor(node) : null;
        const visited = ts.visitEachChild(node, visit, ctx);
        if (!attr) return visited;
        stamped++;
        return f.updateJsxSelfClosingElement(
          visited,
          visited.tagName,
          visited.typeArguments,
          withStamp(visited.attributes, attr),
        );
      }
      return ts.visitEachChild(node, visit, ctx);
    };
    return (root) => ts.visitNode(root, visit) as ts.SourceFile;
  };

  const result = ts.transform(sourceFile, [transformer], { jsx: ts.JsxEmit.Preserve });
  try {
    if (stamped === 0) return source;
    const printer = ts.createPrinter({
      newLine: ts.NewLineKind.LineFeed,
      removeComments: false,
    });
    return printer.printFile(result.transformed[0]);
  } finally {
    result.dispose();
  }
}

/**
 * Register the Bun plugin for the project rooted at `root`. Only the files
 * matched by `designStampScopePattern(root)` are transformed.
 */
export function registerDesignStampPlugin(root: string = process.cwd()): void {
  const filter = designStampScopePattern(root);
  Bun.plugin({
    name: "pylon-design-stamp",
    setup(build) {
      build.onLoad({ filter }, async (args) => {
        const source = await Bun.file(args.path).text();
        const rel = relativeSourcePath(root, args.path);
        return { contents: stampJsxSource(source, rel), loader: "tsx" };
      });
    },
  });
}

// Preload entry: `bun run --preload design-stamp.ts`. Registration is gated on
// the env so importing this module from a test without the flag is a no-op.
if (isDesignMode()) {
  registerDesignStampPlugin(process.cwd());
}
