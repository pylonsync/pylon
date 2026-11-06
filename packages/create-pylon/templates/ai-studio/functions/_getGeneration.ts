import { query, v } from "@pylonsync/functions";

// Internal: read a Generation by id. pollGeneration checks the returned owner
// before it changes the row or calls the provider.
export default query<
  { id: string },
  | {
      id: string;
      userId: string;
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
      | { id: string; userId: string; kind: string; prompt: string; status: string; predictionId?: string | null }
      | null;
    return g;
  },
});
