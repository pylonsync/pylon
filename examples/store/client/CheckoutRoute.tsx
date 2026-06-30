"use client";
/**
 * Client wrapper for the /checkout route — supplies the sync cart + the
 * auth-dialog opener to CheckoutPage, behind the real-account gate.
 */
import React from "react";
import { CheckoutPage } from "./CheckoutPage";
import { RequireAuth } from "./RequireAuth";
import { useCart } from "./lib/cart";
import { openAuth } from "./lib/util";

export function CheckoutRoute() {
  const cart = useCart();
  return (
    <RequireAuth>
      <CheckoutPage cart={cart} onPromptAuth={() => openAuth("login")} />
    </RequireAuth>
  );
}
