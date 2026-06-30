import React from "react";
import type { Metadata } from "@pylonsync/react";
import { RequireAuth } from "../../client/RequireAuth";
import { AccountPage } from "../../client/AccountPage";

export const metadata: Metadata = {
  title: "Your account · Pylon Store",
  description: "Orders + shipping addresses.",
};

// `/account` — orders + addresses for the signed-in user. Gated to real
// accounts; guests get a sign-in prompt.
export default function Page() {
  return (
    <RequireAuth>
      <AccountPage />
    </RequireAuth>
  );
}
