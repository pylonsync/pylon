import { query } from "@pylonsync/functions";
import { providerStatus } from "../lib/provider";

// Whether a model provider key is configured on the server. The UI uses it to
// show setup steps before the first message. It returns two fields and never
// any key material.
export default query({
  auth: "guest",
  args: {},
  async handler(ctx) {
    return providerStatus(ctx.env);
  },
});
