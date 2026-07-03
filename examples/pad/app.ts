/**
 * Pad — collaborative markdown, the whole app.
 *
 * One CRDT entity, two pages, zero extra infrastructure. Every visitor
 * gets a guest session; every document is a shared Loro text — open the
 * same doc in two windows and watch keystrokes merge live, cursors
 * intact. The same single binary serves the SSR pages, the API, the
 * WebSocket fan-out, and the CRDT merge.
 */
import {
  entity,
  field,
  policy,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

const Doc = entity(
  "Doc",
  {
    title: field.string(),
    // The collaborative body. `.crdt("text")` makes this field a Loro
    // text: concurrent edits from different sessions merge character-
    // by-character instead of last-write-wins clobbering.
    content: field.string().crdt("text"),
    createdBy: field.string().readonly(),
    createdAt: field.datetime(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }],
  },
);

// Public demo semantics: anyone can read and any signed-in session
// (guests included) can create and co-edit — that's the point of a pad.
// Deleting stays with the creator.
const docPolicy = policy({
  name: "doc_collab",
  entity: "Doc",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId == data.createdBy",
});

const manifest = buildManifest({
  name: "pad",
  version: "0.1.0",
  entities: [Doc],
  queries: [],
  actions: [],
  policies: [docPolicy],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));
