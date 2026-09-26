import { query } from "@pylonsync/functions";
import { findOwner } from "../lib/store";

// The owner's user id (lib/store.ts findOwner). Internal: the agent runs as
// this account, so the id never leaves the server.
export default query({
  internal: true,
  auth: "admin",
  args: {},
  async handler(ctx) {
    return findOwner(ctx);
  },
});
