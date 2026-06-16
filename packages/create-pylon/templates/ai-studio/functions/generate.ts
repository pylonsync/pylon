import { mutation, v } from "@pylonsync/functions";

// generate — kick off a generation. A `mutation`: it inserts a pending row
// (stamped to the caller) and enqueues the `pollGeneration` background job, then
// returns. It does NO network I/O itself — generation runs in the job, off the
// request path, so a slow model (video can take minutes) never blocks or times
// out the call. The pending row syncs to the gallery instantly; the job flips it
// to the result when it's ready.
//
// `auth: "user"` — generating requires a signed-in account (the page already
// redirects anyone else to /login).
export default mutation<{ kind: string; prompt: string }, { id: string }>({
  auth: "user",
  args: { kind: v.string(), prompt: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Sign in to generate.");
    const kind = args.kind;
    const prompt = args.prompt.trim();
    if (!["image", "audio", "video"].includes(kind)) {
      throw ctx.error("INVALID_ARGS", "kind must be image, audio, or video.");
    }
    if (prompt.length < 2 || prompt.length > 1000) {
      throw ctx.error("INVALID_ARGS", "Enter a prompt (up to 1000 characters).");
    }

    const id = await ctx.db.unsafe.insert("Generation", {
      userId,
      kind,
      prompt,
      status: "pending",
      demo: false,
      createdAt: new Date().toISOString(),
    });

    // Enqueue the background job (deferred until this mutation commits). It
    // owns the provider call + polling + the final update.
    await ctx.scheduler.runAfter(0, "pollGeneration", { id });

    return { id };
  },
});
