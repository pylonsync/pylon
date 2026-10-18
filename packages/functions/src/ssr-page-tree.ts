// The shape of the tree around a route's page, shared by the server render
// (ssr-runtime.ts) and the browser's hydration (the generated client runtime
// imports this file as `./page-tree`).
//
// React derives `useId` values from the tree's shape, including how many
// children each element has. The server renders the page's metadata (<title>,
// <meta>, <link>, which React hoists into <head>) beside the page; the browser
// never renders those tags. So both sides give the page a fragment with two
// slots: on the server the first slot holds the metadata, in the browser it
// holds null. React counts a null slot, so every id inside the page is the
// same on both sides. Without the slot, every Radix menu, dialog and label id
// on a page with metadata mismatched on hydration.
//
// No imports: the client bundler copies this file next to the runtime.

type CreateElement = (type: any, props: any, ...children: any[]) => any;

/** The page with its metadata slot. `meta` is null in the browser. */
export function withMetaSlot(createElement: CreateElement, Fragment: any, meta: unknown, page: unknown): any {
  return createElement(Fragment, null, meta ?? null, page);
}
