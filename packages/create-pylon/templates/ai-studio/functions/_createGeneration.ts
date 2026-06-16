import { mutation, v } from "@pylonsync/functions";

// Internal: insert a pending Generation stamped to the caller, return its id.
// Called only by the generate action (internal:true → not client-reachable), so
// it trusts its args. Writing here (not from the client) is why Generation is
// allowInsert:"false" — the gallery is read-only to clients, written only by the
// server-side generate pipeline. The pending row syncs to the user's tabs
// immediately, so a "generating…" card appears before the provider responds.
export default mutation<
  { kind: string; prompt: string },
  { id: string }
>({
  internal: true,
  args: { kind: v.string(), prompt: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "A session is required to generate.");
    const id = await ctx.db.unsafe.insert("Generation", {
      userId,
      kind: args.kind,
      prompt: args.prompt,
      status: "pending",
      demo: false,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
