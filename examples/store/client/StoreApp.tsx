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
import { useEffect, useState } from "react";
import { init, callFn } from "@pylonsync/react";
import { ShoppingCart, User as UserIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Catalog } from "./Catalog";
import { ProductDetail } from "./ProductDetail";
import { AccountPage } from "./AccountPage";
import { CheckoutPage } from "./CheckoutPage";
import { OrderDetail } from "./OrderDetail";
import { CartSheet } from "./CartSheet";
import { AuthDialog } from "./AuthDialog";
import { UserMenu } from "./UserMenu";
import { ensureGuestSession, useAuth } from "./lib/auth";
import { useCart } from "./lib/cart";
import { navigate } from "./lib/util";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
const WS_URL =
  import.meta.env.VITE_PYLON_WS_URL ??
  (BASE_URL.startsWith("https://")
    ? `${BASE_URL.replace(/^https:/, "wss:").replace(/\/$/, "")}:4322`
    : undefined);

init({ baseUrl: BASE_URL, appName: "store", wsUrl: WS_URL });

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

  // Bootstrap a guest session if the user isn't logged in. The seed
  // function needs `auth.userId`, and per-user CartItem policies
  // need a stable id even for anonymous browsing.
  useEffect(() => {
    (async () => {
      await ensureGuestSession();
      callFn("seedCatalog", { count: 10_000 }).catch(() => {});
    })();
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
    <header className="sticky top-0 z-30 flex h-14 items-center gap-4 border-b bg-background/95 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/80 md:px-6">
      <button
        className="flex items-center gap-2 font-semibold text-primary"
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
