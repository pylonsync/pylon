import { query } from "@statecraft/functions";

export default query({
  args: {},
  async handler(ctx) {
    const all = await ctx.db.list("Listing");
    return all.filter((l) => !l.settledAt && new Date(l.endsAt).getTime() > Date.now());
  },
});
