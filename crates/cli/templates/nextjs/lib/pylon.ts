import { createPylonServer } from "@pylonsync/next";

// Centralized server-side Pylon helpers. Import `pylon` anywhere in
// your Server Components / Route Handlers / Server Actions to:
//
//   const auth = await pylon.requireAuth();             // 401 → /login
//   const posts = await pylon.json<Post[]>("/api/entities/Post");
//
// `cookieName` MUST match what the Pylon backend sets. Pylon's default
// is `${app_name}_session`; this value matches what's in apps/api/app.ts.
export const pylon = createPylonServer({
  cookieName: "__APP_NAME___session",
  target: process.env.PYLON_TARGET ?? "http://localhost:4321",
  loginUrl: "/login",
});
