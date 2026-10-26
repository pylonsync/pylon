// The shape of the tree around a route's page, shared by the server render
// (ssr-runtime.ts) and the browser's hydration (the generated client runtime
// imports this file as `./page-tree`).
//
// React derives `useId` values from the tree's shape, including how many
// children each element has. Both sides give the page a fragment with two
// slots, the first one null. The server once put the page's metadata there;
// it now writes the metadata into <head> as HTML, outside the tree. The slot
// stays so the tree keeps the shape that built clients hydrate against.
//
// No imports: the client bundler copies this file next to the runtime.

type CreateElement = (type: any, props: any, ...children: any[]) => any;

/** The page with its metadata slot. Both sides pass null for `meta`. */
export function withMetaSlot(createElement: CreateElement, Fragment: any, meta: unknown, page: unknown): any {
  return createElement(Fragment, null, meta ?? null, page);
}
