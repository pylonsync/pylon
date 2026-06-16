import { query, v } from "@pylonsync/functions";

// Internal: read a Generation by id, bypassing the owner read-policy (the
// pollGeneration job runs without a user session). Returns the fields the job
// needs, or null.
export default query<
  { id: string },
  | {
      id: string;
      kind: string;
      prompt: string;
      status: string;
      predictionId?: string | null;
    }
  | null
>({
  internal: true,
  args: { id: v.id("Generation") },
  async handler(ctx, args) {
    const g = (await ctx.db.unsafe.get("Generation", args.id)) as
      | { id: string; kind: string; prompt: string; status: string; predictionId?: string | null }
      | null;
    return g;
  },
});
