import { type RouteHandler } from "@pylonsync/react";

// `app/notes/route.ts` → handles POST /notes (the same path the page lives at).
// A `<Form action="/notes">` posts here; we create a Note, then 303-redirect
// back to /notes (POST-redirect-GET) so a no-JS browser re-renders with the new
// note. Validation errors round-trip through a query param. With JS, the
// runtime intercepts the submit + swaps the page in without a full reload —
// same handler, no extra code.
//
// CSRF is automatic: Pylon's Origin gate rejects cross-site POSTs before this
// runs, and the session cookie is SameSite=Lax. Writes here use the normal
// function trust model — gate sensitive actions on `auth` inside the handler.
export const POST: RouteHandler = async ({ form, db, response }) => {
  const body = (form.get("body") ?? "").trim();
  if (!body) {
    response.redirect("/notes?error=" + encodeURIComponent("Note can't be empty"));
    return;
  }
  await db.insert("Note", { body });
  response.redirect("/notes?created=1");
};
