export function hashColor(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  const hue = Math.abs(h) % 360;
  return `hsl(${hue}, 50%, 55%)`;
}

export function gradient(a: string, b: string) {
  return `linear-gradient(135deg, ${hashColor(a)}, ${hashColor(b)})`;
}

export function initials(name: string) {
  return name
    .split(" ")
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

export function formatPrice(n: number) {
  return `$${n.toFixed(2)}`;
}

// Canonical path for a product's SSR detail page. Human-readable + shareable.
export function productPath(slug: string) {
  return `/p/${slug}`;
}

// Programmatic client-side navigation to a real route. Uses Pylon's router
// (window.__pylon) so it works from any event handler without a hook; falls
// back to a hard navigation before the router has mounted.
export function navigate(href: string) {
  if (typeof window === "undefined") return;
  const pylon = (window as unknown as {
    __pylon?: { navigate?: (href: string, opts?: { push?: boolean }) => void };
  }).__pylon;
  if (pylon?.navigate) pylon.navigate(href, { push: true });
  else window.location.assign(href);
}

// The auth dialog lives in the layout-level <StoreChrome> island, but any
// route (a checkout sign-in wall, the header) needs to open it. Rather than
// thread props across the router boundary, pages fire this event and the
// chrome listens — the same lightweight cross-component signal the auth/cart
// hooks already use.
export const OPEN_AUTH_EVENT = "store-open-auth";

export function openAuth(mode: "login" | "register" = "login") {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent(OPEN_AUTH_EVENT, { detail: { mode } }));
}
