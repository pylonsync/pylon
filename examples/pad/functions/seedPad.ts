import { mutation } from "@pylonsync/functions";

/**
 * First-boot seed: create the welcome document so the pad is never an
 * empty shell. Public because the first visitor is a guest; the
 * advisory lock guards a double-seed from two windows racing on first
 * load.
 */
const WELCOME = `# Welcome to Pad

A collaborative markdown editor — the whole app is **one entity, two pages,
and one Pylon binary** serving SSR, the API, the WebSocket fan-out, and the
CRDT merge.

## Try it

1. Copy this page's URL
2. Open it in a second window (or send it to a friend)
3. Type in either window

Keystrokes merge character-by-character through a [Loro](https://loro.dev)
text CRDT — no last-write-wins clobbering, carets stay put.

## How it works

The body of this document is a single schema field:

\`\`\`ts
const Doc = entity("Doc", {
  title: field.string(),
  content: field.string().crdt("text"),
});
\`\`\`

And the editor is one hook:

\`\`\`tsx
const { ref, value, onInput } = useCollabTextarea("Doc", id, "content");
\`\`\`

> Everything you see — auth, sync, policies, this page's server render —
> comes from \`pylon dev\` on a single port.

Built with [Pylon](https://www.pylonsync.com). Source: \`examples/pad\`.
`;

export default mutation<
  Record<string, never>,
  { seeded: boolean; id?: string }
>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("pad_seed");
    const existing = await ctx.db.unsafe.list("Doc");
    if (existing.length > 0) return { seeded: false };

    const now = new Date().toISOString();
    const id = await ctx.db.unsafe.insert("Doc", {
      title: "Welcome to Pad",
      content: WELCOME,
      createdBy: "system",
      createdAt: now,
      updatedAt: now,
    });
    return { seeded: true, id };
  },
});
