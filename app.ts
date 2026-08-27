/**
 * pylonsync.com — the Pylon framework marketing site.
 *
 * Split out of the control-plane app so the two brands stop sharing one
 * deployment. A Pylon app resolves exactly ONE canonical host, so serving
 * pylonsync.com (framework) and usesmallware.com (product) from a single app
 * forced a choice between letting both hosts self-canonicalize identical
 * content and teaching the framework to branch on hostname. Two apps, two
 * canonical hosts, no framework change.
 *
 * The other reason is blast radius: a landing-page copy edit used to redeploy
 * the app that provisions customer machines, holds every session, and runs
 * billing. Marketing is the highest-frequency, lowest-risk change in the repo
 * and no longer carries that risk.
 *
 * NO ENTITIES, NO POLICIES, NO DATABASE. This app renders pages and nothing
 * else, so there is no schema to migrate, no volume to mount, and no
 * DATABASE_URL to set. The two functions it does declare are stateless
 * relays — see functions/.
 */
import { auth, buildManifest, discoverAppRoutes, font } from "@pylonsync/sdk";

const manifest = buildManifest({
  name: "pylonsync-site",
  version: "0.1.0",
  // Deliberately empty. Anything that needs to persist belongs to the control
  // plane, which owns the accounts, the billing, and the signup list; this app
  // reaches it over HTTP (functions/joinUpdates.ts) rather than growing a
  // second copy of the data.
  entities: [],
  policies: [],
  queries: [],
  actions: [],
  crons: [],
  routes: await discoverAppRoutes({ appDir: "web/app" }),
  // Geist only. The Stack0 Cloud product faces (Archivo / Newsreader / IBM Plex
  // Mono) stay with the control plane — this app never renders that brand, so
  // shipping their @font-face blocks here would be dead weight on every page.
  fonts: [
    font({ family: "Geist", variable: "--font-geist-sans", weights: ["300..700"] }),
    font({ family: "Geist Mono", variable: "--font-geist-mono", weights: ["400..600"] }),
  ],
  // There is no sign-in here and no session to protect, but the manifest still
  // needs `trustedOrigins`: it is the declarative source for the CORS gate,
  // which refuses to start on a wildcard in production. These are simply the
  // origins this app is served from. The footer signup posts to this app's OWN
  // origin and is relayed server-side (functions/joinUpdates.ts), so it needs
  // no cross-origin entry.
  auth: auth({
    trustedOrigins: [
      "https://www.pylonsync.com",
      "https://pylonsync.com",
      "https://pylonsync-site.fly.dev",
    ],
  }),
});

console.log(JSON.stringify(manifest, null, 2));
