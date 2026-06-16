import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
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
// Multi-user: every signed-in (or guest) visitor gets their own private studio.
// Image + audio call OpenAI when OPENAI_API_KEY is set; with no key the studio
// returns a clearly-labeled placeholder so the whole flow + live gallery work
// with zero config. Video is a stubbed extension point — see functions/generate.ts.
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

// Generations are PRIVATE per user: you can READ only your own, and you can't
// write them from the client at all — the generate action (which runs the
// provider call with the server-side key) is the only writer. So the gallery is
// live (the sync engine ships you your rows as the action updates them) without
// ever exposing one user's generations to another.
const generationPolicy = policy({
  name: "generation_owner_read",
  entity: "Generation",
  allowRead: "auth.userId == data.userId",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
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
  // generate (public action) + _createGeneration / _finishGeneration (internal
  // mutations it calls) live in functions/ and are discovered automatically.
  queries: [],
  actions: [],
  policies: [generationPolicy, userPolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
