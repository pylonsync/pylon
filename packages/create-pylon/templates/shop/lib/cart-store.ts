// A tiny client-side cart store shared by the product grid (app/(marketing)/
// shop-client.tsx) and the nav cart button (components/cart-button.tsx). The
// two live in different client islands, so React state can't be lifted between
// them; this module holds the cart and both islands subscribe with
// `useSyncExternalStore`. Cart contents are slug → qty; the grid clamps them to
// live stock when it renders.

import { useSyncExternalStore } from "react";

export type CartLines = Record<string, number>;

type CartState = { lines: CartLines; open: boolean };

let state: CartState = { lines: {}, open: false };
const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot() {
  return state;
}

// The server render has no cart; every island sees the same empty snapshot.
const SERVER_STATE: CartState = { lines: {}, open: false };
function getServerSnapshot() {
  return SERVER_STATE;
}

export function useCart() {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export const cart = {
  setLines(update: (lines: CartLines) => CartLines) {
    state = { ...state, lines: update(state.lines) };
    emit();
  },
  open() {
    state = { ...state, open: true };
    emit();
  },
  close() {
    state = { ...state, open: false };
    emit();
  },
};

export function countItems(lines: CartLines) {
  return Object.values(lines).reduce((s, q) => s + q, 0);
}
