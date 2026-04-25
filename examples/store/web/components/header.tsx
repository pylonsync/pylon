"use client";

import Link from "next/link";
import { useState } from "react";
import { ShoppingCart, User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { useAuth } from "@/lib/pylon-client";
import { CartSheet } from "./cart-sheet";
import { AuthDialog } from "./auth-dialog";
import { UserMenu } from "./user-menu";
import { CartBadge } from "./cart-badge";

export function Header() {
  const { user, isAuthenticated } = useAuth();
  const [cartOpen, setCartOpen] = useState(false);
  const [authOpen, setAuthOpen] = useState(false);
  const [authMode, setAuthMode] = useState<"login" | "register">("login");

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-4 border-b bg-background/95 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/80 md:px-6">
      <Link
        href="/"
        className="flex items-center gap-2 font-semibold text-primary"
      >
        <BrandMark />
        <span>Pylon Store</span>
      </Link>

      <div className="flex-1" />

      {isAuthenticated && user ? (
        <UserMenu user={user} />
      ) : (
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setAuthMode("login");
              setAuthOpen(true);
            }}
          >
            <UserIcon className="size-4" />
            Log in
          </Button>
          <Button
            size="sm"
            onClick={() => {
              setAuthMode("register");
              setAuthOpen(true);
            }}
          >
            Sign up
          </Button>
        </div>
      )}

      <Button
        variant="outline"
        size="icon"
        onClick={() => setCartOpen(true)}
        aria-label="Open cart"
        className="relative"
      >
        <ShoppingCart className="size-4" />
        <CartBadge />
      </Button>

      <CartSheet open={cartOpen} onClose={() => setCartOpen(false)} />
      <AuthDialog
        open={authOpen}
        mode={authMode}
        onModeChange={setAuthMode}
        onClose={() => setAuthOpen(false)}
      />
    </header>
  );
}

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
