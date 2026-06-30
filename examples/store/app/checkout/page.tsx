import React from "react";
import type { Metadata } from "@pylonsync/react";
import { CheckoutRoute } from "../../client/CheckoutRoute";

export const metadata: Metadata = {
  title: "Checkout · Pylon Store",
  description: "Pick an address and place your order.",
};

// `/checkout` — address picker + place order. Real-account gated inside
// CheckoutRoute.
export default function Page() {
  return <CheckoutRoute />;
}
