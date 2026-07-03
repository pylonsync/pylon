/**
 * Marker import that pins a module to the SERVER. Put it at the top of any
 * module that must never reach the browser — one holding secrets, server
 * config, or node-only APIs:
 *
 *   import "@pylonsync/functions/server-only";
 *
 *   export const stripeKey = process.env.STRIPE_SECRET_KEY!;
 *
 * Page (`page.tsx`) and layout (`layout.tsx`) modules — and everything they
 * transitively import — are bundled for client hydration, so a plain literal or
 * server config in that graph would ship to the browser. If a module marked
 * with this import is pulled into a client-reachable page, the SSR client
 * bundler REFUSES to build and names the offending importer (see the
 * `pylon-server-only` plugin in `ssr-client-bundler.ts`).
 *
 * On the server this is an inert no-op. There is nothing to call — importing it
 * is the whole contract.
 */
export {};
