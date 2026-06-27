import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// ai-studio — a generative media studio (image / audio / video). The realtime
// hook is the generation gallery: kick off a generation and a "generating…"
// card appears instantly, then flips to the finished result the moment the
// server-side `generate` action resolves — live, across every open tab. The
// provider call (and your API key) stays on the server.
//
// Two entities:
//   • Generation — one generation request + its result. Owner-scoped: you only
//                  see your own. Written ONLY by the generate action (clients
//                  can't insert), and read live via `db.useQuery`.
//   • User       — the account (email/password is built in).
//
// Multi-user: every signed-in visitor gets their own private studio. Image,
// audio, AND video generate via Replicate when REPLICATE_API_TOKEN is set; with
// no token the studio returns a clearly-labeled placeholder so the whole flow +
// background job + live gallery work with zero config (see functions/).
// ---------------------------------------------------------------------------

const Generation = entity(
  "Generation",
  {
    // Stamped from the session inside the generate action (server-side), not by
    // the client — so it's an owner-scoped READ, with no client writes at all.
    userId: field.string(),
    kind: field.string(), // "image" | "audio" | "video"
    prompt: field.string(),
    status: field.string().default("pending"), // "pending" | "processing" | "done" | "failed"
    // The result: an image/audio/video URL (or data: URL), ready to drop into
    // an <img>/<audio>/<video>.
    resultUrl: field.string().optional(),
    error: field.string().optional(),
    // The Replicate prediction id, while a generation is "processing" — the
    // client polls checkGeneration with the row's id until it settles.
    predictionId: field.string().optional(),
    // True when this is the no-API-key placeholder (the UI shows a "demo" badge).
    demo: field.boolean().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
);

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// Generations are PRIVATE per user: read-your-own, with no client writes at all
// (the server-side pipeline is the only writer). The gallery stays live via the
// sync engine without ever exposing one user's generations to another.
const generationPolicy = policy({
  name: "generation_owner_read",
  entity: "Generation",
  allowRead: "auth.userId == data.userId",
  allowInsert: "false", // created only by the generate pipeline (server-side)
  allowUpdate: "false", // updated only by the pollGeneration job
  allowDelete: "auth.userId == data.userId", // you can delete your own from the gallery
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Generation, User],
  // generate (mutation) + pollGeneration (job) + the internal _getGeneration /
  // _updateGeneration live in functions/ and are discovered automatically.
  queries: [],
  actions: [],
  policies: [generationPolicy, userPolicy],
  auth: auth(),
  // Self-hosted Inter (next/font parity): the build fetches the woff2, serves it
  // same-origin (no third-party request, no FOUT), preloads it, and synthesizes a
  // size-adjusted fallback face so there's no layout shift. globals.css reads it
  // via `var(--font-sans, …)`; layout.tsx carries no font <link>.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
