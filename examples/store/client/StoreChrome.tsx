"use client";
/**
 * Persistent client chrome for the store, mounted once in app/layout.tsx so
 * it survives client-side route transitions. Hosts the singletons that must
 * be shared across every route: the sync-engine bootstrap, the header (cart
 * count + auth), the cart drawer, and the auth dialog.
 *
 * Cart/auth state isn't passed via context — `useCart`/`useAuth` read the
 * sync store + localStorage directly, so any component on any route sees the
 * same state. The only cross-route signal is "open the auth dialog", which
 * pages fire via `openAuth()` (a window event this island listens for).
 */
import React, { useEffect, useState } from "react";
import {
  Link,
  callFn,
  configureClient,
  init,
  storageKey,
} from "@pylonsync/react";
import { ShoppingCart, User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Badge } from "@pylonsync/example-ui/badge";
import { CartSheet } from "./CartSheet";
import { AuthDialog } from "./AuthDialog";
import { UserMenu } from "./UserMenu";
import { useAuth } from "./lib/auth";
import { useCart } from "./lib/cart";
import { OPEN_AUTH_EVENT } from "./lib/util";

const APP_NAME = "store";

// Boot the sync engine + establish a guest session so unauthenticated
// browsing works. The catalog reads a public-read table, so search renders
// without waiting on this — it just enables cart/auth.
async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id)
      window.localStorage.setItem(storageKey("userId"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon not reachable yet — hooks retry.
  }
}

export function StoreChrome() {
  const cart = useCart();
  const [booted, setBooted] = useState(false);
  const [cartOpen, setCartOpen] = useState(false);
  const [authOpen, setAuthOpen] = useState(false);
  const [authMode, setAuthMode] = useState<"login" | "register">("login");

  useEffect(() => {
    void bootstrap().then(() => {
      setBooted(true);
      // Seed the 10k-product catalog on first launch (idempotent — skips once
      // the target row count exists). Lives here so it runs once globally,
      // regardless of which route the visitor lands on.
      void callFn("seedCatalog", { count: 10_000 }).catch(() => {});
    });
  }, []);

  // Any route can ask to open the auth dialog (checkout/account sign-in wall).
  useEffect(() => {
    const onOpen = (e: Event) => {
      const mode = (e as CustomEvent<{ mode?: "login" | "register" }>).detail
        ?.mode;
      setAuthMode(mode ?? "login");
      setAuthOpen(true);
    };
    window.addEventListener(OPEN_AUTH_EVENT, onOpen);
    return () => window.removeEventListener(OPEN_AUTH_EVENT, onOpen);
  }, []);

  return (
    <>
      <Header
        booted={booted}
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
      <CartSheet open={cartOpen} onClose={() => setCartOpen(false)} cart={cart} />
      <AuthDialog
        open={authOpen}
        mode={authMode}
        onModeChange={setAuthMode}
        onClose={() => setAuthOpen(false)}
      />
    </>
  );
}

function Header({
  booted,
  onOpenCart,
  onLogin,
  onSignup,
}: {
  booted: boolean;
  onOpenCart: () => void;
  onLogin: () => void;
  onSignup: () => void;
}) {
  const { user, isAuthenticated } = useAuth();
  const cart = useCart();

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-4 border-b bg-background px-5">
      <Link
        href="/"
        className="flex items-center gap-2 text-sm font-medium text-foreground transition-colors hover:text-primary"
      >
        <BrandMark />
        <span>Pylon Store</span>
      </Link>

      <div className="flex-1" />

      {booted && isAuthenticated && user ? (
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
        {cart.count > 0 && (
          <Badge
            variant="default"
            className="absolute -right-1.5 -top-1.5 h-5 min-w-5 justify-center rounded-full px-1 text-[10px]"
          >
            {cart.count}
          </Badge>
        )}
      </Button>
    </header>
  );
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="18" height="24" fill="currentColor" aria-hidden>
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}
