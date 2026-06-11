"use client";
/**
 * Pylon Store — full e-commerce showcase.
 *
 * Routing is plain `window.location.hash` since the demo only has
 * a handful of routes:
 *
 *   #/                — catalog (faceted search)
 *   #/p/<id>          — product detail
 *   #/account         — orders + addresses
 *   #/checkout        — address picker + place order
 *   #/orders/<id>     — order detail with shipping timeline
 *
 * Auth + cart are global concerns hosted at this level so every
 * route shares the same singletons (one cart drawer, one auth
 * dialog, one header).
 */
import React, { useEffect, useState } from "react";
import { callFn } from "@pylonsync/react";
import { ShoppingCart, User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Badge } from "@pylonsync/example-ui/badge";
import { Catalog } from "./Catalog";
import { ProductDetail } from "./ProductDetail";
import { AccountPage } from "./AccountPage";
import { CheckoutPage } from "./CheckoutPage";
import { OrderDetail } from "./OrderDetail";
import { CartSheet } from "./CartSheet";
import { AuthDialog } from "./AuthDialog";
import { UserMenu } from "./UserMenu";
import { useAuth } from "./lib/auth";
import { useCart } from "./lib/cart";
import { navigate } from "./lib/util";

// ---------------------------------------------------------------------------
// Hash routing
// ---------------------------------------------------------------------------

type Route =
  | { name: "catalog" }
  | { name: "product"; id: string }
  | { name: "account" }
  | { name: "checkout" }
  | { name: "order"; id: string };

function parseHash(): Route {
  const hash = window.location.hash || "#/";
  const product = hash.match(/^#\/p\/([^/?#]+)/);
  if (product) return { name: "product", id: decodeURIComponent(product[1]) };
  const order = hash.match(/^#\/orders\/([^/?#]+)/);
  if (order) return { name: "order", id: decodeURIComponent(order[1]) };
  if (hash.startsWith("#/account")) return { name: "account" };
  if (hash.startsWith("#/checkout")) return { name: "checkout" };
  return { name: "catalog" };
}

function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parseHash());
  useEffect(() => {
    const onHash = () => setRoute(parseHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);
  return route;
}

// ---------------------------------------------------------------------------
// Top-level shell
// ---------------------------------------------------------------------------

export function StoreApp() {
  const route = useRoute();
  const auth = useAuth();
  const cart = useCart();
  const [cartOpen, setCartOpen] = useState(false);
  const [authOpen, setAuthOpen] = useState(false);
  const [authMode, setAuthMode] = useState<"login" | "register">("login");

  // Seed the catalog in the background after initial mount. The function
  // is idempotent — it skips if ≥ target rows already exist.
  useEffect(() => {
    callFn("seedCatalog", { count: 10_000 }).catch(() => {});
  }, []);

  // Routes that require a real (non-guest) account. If a guest hits
  // these, prompt them to log in or sign up rather than loading them
  // into a half-broken page.
  const requiresAuth = route.name === "account" || route.name === "checkout";
  const blocked = requiresAuth && !auth.isAuthenticated;

  return (
    <div className="flex min-h-screen flex-col">
      <Header
        cartCount={cart.count}
        onOpenCart={() => setCartOpen(true)}
        onLogin={() => {
          setAuthMode("login");
          setAuthOpen(true);
        }}
        onSignup={() => {
          setAuthMode("register");
          setAuthOpen(true);
        }}
      />

      <div className="flex-1">
        {blocked ? (
          <SignInWall
            onLogin={() => {
              setAuthMode("login");
              setAuthOpen(true);
            }}
            onSignup={() => {
              setAuthMode("register");
              setAuthOpen(true);
            }}
          />
        ) : route.name === "catalog" ? (
          <Catalog onAddToCart={cart.add} />
        ) : route.name === "product" ? (
          <ProductDetail id={route.id} onAddToCart={cart.add} />
        ) : route.name === "account" ? (
          <AccountPage />
        ) : route.name === "checkout" ? (
          <CheckoutPage
            cart={cart}
            onPromptAuth={() => setAuthOpen(true)}
          />
        ) : (
          <OrderDetail id={route.id} />
        )}
      </div>

      <CartSheet
        open={cartOpen}
        onClose={() => setCartOpen(false)}
        cart={cart}
      />

      <AuthDialog
        open={authOpen}
        mode={authMode}
        onModeChange={setAuthMode}
        onClose={() => setAuthOpen(false)}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

function Header({
  cartCount,
  onOpenCart,
  onLogin,
  onSignup,
}: {
  cartCount: number;
  onOpenCart: () => void;
  onLogin: () => void;
  onSignup: () => void;
}) {
  const { user, isAuthenticated } = useAuth();

  return (
    <header className="sticky top-0 z-30 h-14 border-b bg-background px-5 flex items-center gap-4">
      <button
        className="flex items-center gap-2 font-medium text-sm text-foreground hover:text-primary transition-colors"
        onClick={() => navigate("#/")}
      >
        <BrandMark />
        <span>Pylon Store</span>
      </button>

      <div className="flex-1" />

      {isAuthenticated && user ? (
        <UserMenu user={user} />
      ) : (
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onLogin}>
            <UserIcon className="size-4" />
            Log in
          </Button>
          <Button size="sm" onClick={onSignup}>
            Sign up
          </Button>
        </div>
      )}

      <Button
        variant="outline"
        size="icon"
        onClick={onOpenCart}
        aria-label="Open cart"
        className="relative"
      >
        <ShoppingCart className="size-4" />
        {cartCount > 0 && (
          <Badge
            variant="default"
            className="absolute -right-1.5 -top-1.5 h-5 min-w-5 justify-center rounded-full px-1 text-[10px]"
          >
            {cartCount}
          </Badge>
        )}
      </Button>
    </header>
  );
}

// ---------------------------------------------------------------------------
// Auth wall for /account and /checkout
// ---------------------------------------------------------------------------

function SignInWall({
  onLogin,
  onSignup,
}: {
  onLogin: () => void;
  onSignup: () => void;
}) {
  return (
    <main className="mx-auto flex max-w-md flex-col items-center gap-4 p-8 text-center">
      <UserIcon className="size-12 text-muted-foreground" />
      <h2 className="text-xl font-semibold">Sign in to continue</h2>
      <p className="text-sm text-muted-foreground">
        Your cart, orders, and shipping details live with your account. Create
        one in 10 seconds — no email verification required for the demo.
      </p>
      <div className="flex w-full gap-2">
        <Button className="flex-1" onClick={onSignup}>
          Sign up
        </Button>
        <Button className="flex-1" variant="outline" onClick={onLogin}>
          Log in
        </Button>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// Logo
// ---------------------------------------------------------------------------

function BrandMark() {
  return (
    <svg
      viewBox="0 0 48 64"
      width="18"
      height="24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}
