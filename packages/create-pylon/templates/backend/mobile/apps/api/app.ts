import {
  auth,
  buildManifest,
  discoverFunctions,
  entity,
  field,
  policy,
} from "@pylonsync/sdk";
// Native in-app purchases: RevenueCat events become RcEntitlement rows
// that sync to every device. See lib/purchases.ts.
import { purchases, FREE_NOTE_LIMIT } from "./lib/purchases";

// Accounts. Email code, Sign in with Apple, and Google Sign-In all land
// here. The app starts every user as a guest; signing in later merges the
// guest's rows into the account, so nothing made before sign-up is lost.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    // The register / OAuth paths stamp these; the entity must declare them.
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// The app's data. One owner-scoped entity to build on: replace `Note` with
// whatever your app stores. `field.owner()` stamps the caller's user id on
// insert and the policy scopes every read and write to that owner.
const Note = entity(
  "Note",
  {
    ownerId: field.id("User").owner(),
    title: field.string(),
    body: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_owner", fields: ["ownerId"], unique: false }] },
);

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "auth.userId == data.id",
  allowDelete: "false",
});

// Reads, edits, and deletes go straight through sync. Inserts go through
// the `createNote` action so the free-tier cap is enforced on the server
// (a client cannot skip the paywall by writing the row itself).
const notePolicy = policy({
  name: "note_owner",
  entity: "Note",
  allowRead: "auth.userId == data.ownerId",
  allowInsert: "false",
  allowUpdate: "auth.userId == data.ownerId",
  allowDelete: "auth.userId == data.ownerId",
});

const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [User, Note, ...purchases.manifest.entities],
  queries: fns.queries,
  // The plugin's handlers are re-exported from functions/, so discovery
  // already lists them; spreading `purchases.manifest.actions` too would
  // register each twice.
  actions: fns.actions,
  policies: [userPolicy, notePolicy, ...purchases.manifest.policies],
  // API only: the Expo app is the frontend.
  routes: [],
  // Email/password + magic codes are on by default. Native sign-in needs
  // the app's ids on the server: PYLON_APPLE_NATIVE_CLIENT_IDS (the iOS
  // bundle id) and PYLON_GOOGLE_NATIVE_CLIENT_IDS (the iOS + Android OAuth
  // client ids). See .env.example.
  auth: auth(),
});

// The CLI runs `bun run app.ts` and reads this as the manifest.
console.log(JSON.stringify(manifest, null, 2));

export { FREE_NOTE_LIMIT };
export default manifest;
