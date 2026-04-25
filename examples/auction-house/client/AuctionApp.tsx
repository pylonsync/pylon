/**
 * Pylon Auction House — timed + live auctions with many lots.
 *
 * Hash routes:
 *   #/                  — auction list (homepage)
 *   #/a/<id>            — auction detail (lot grid)
 *   #/a/<id>/live       — live auction room
 *   #/lot/<id>          — single timed lot with bidding
 *   #/account           — my bids + watchlist
 *
 * Auth + bidder identity are global; the Header handles them across
 * every route.
 */
import { useEffect, useState } from "react";
import { init, callFn } from "@pylonsync/react";
import { Gavel, ShieldCheck, User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { AuctionList } from "./AuctionList";
import { AuctionDetail } from "./AuctionDetail";
import { LiveRoom } from "./LiveRoom";
import { LotDetail } from "./LotDetail";
import { Account } from "./Account";
import { CreateAuctionDialog } from "./CreateAuctionDialog";
import { AuthDialog } from "./AuthDialog";
import { UserMenu } from "./UserMenu";
import { ensureGuestSession, useAuth } from "./lib/auth";
import { navigate } from "./lib/util";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "auction-house" });

type Route =
  | { name: "list" }
  | { name: "auction"; id: string }
  | { name: "live"; id: string }
  | { name: "lot"; id: string }
  | { name: "account" };

function parseHash(): Route {
  const hash = window.location.hash || "#/";
  const live = hash.match(/^#\/a\/([^/?#]+)\/live/);
  if (live) return { name: "live", id: decodeURIComponent(live[1]) };
  const auction = hash.match(/^#\/a\/([^/?#]+)/);
  if (auction) return { name: "auction", id: decodeURIComponent(auction[1]) };
  const lot = hash.match(/^#\/lot\/([^/?#]+)/);
  if (lot) return { name: "lot", id: decodeURIComponent(lot[1]) };
  if (hash.startsWith("#/account")) return { name: "account" };
  return { name: "list" };
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

export function AuctionApp() {
  const route = useRoute();
  const auth = useAuth();
  const [authOpen, setAuthOpen] = useState(false);
  const [authMode, setAuthMode] = useState<"login" | "register">("login");
  const [createOpen, setCreateOpen] = useState(false);

  // Bootstrap a guest session and seed sample auctions on first load.
  useEffect(() => {
    (async () => {
      await ensureGuestSession();
      callFn("seedAuctionHouse", {}).catch(() => {});
    })();
  }, []);

  const requiresAuth = route.name === "account";
  const blocked = requiresAuth && !auth.isAuthenticated;

  return (
    <div className="flex min-h-screen flex-col">
      <Header
        onOpenAuth={(mode) => {
          setAuthMode(mode);
          setAuthOpen(true);
        }}
        onCreate={() => {
          if (!auth.isAuthenticated) {
            setAuthMode("register");
            setAuthOpen(true);
            return;
          }
          setCreateOpen(true);
        }}
      />

      <main className="flex-1">
        {blocked ? (
          <SignInWall onPrompt={() => setAuthOpen(true)} />
        ) : route.name === "list" ? (
          <AuctionList />
        ) : route.name === "auction" ? (
          <AuctionDetail id={route.id} />
        ) : route.name === "live" ? (
          <LiveRoom id={route.id} />
        ) : route.name === "lot" ? (
          <LotDetail
            id={route.id}
            onPromptAuth={() => {
              setAuthMode("login");
              setAuthOpen(true);
            }}
          />
        ) : (
          <Account />
        )}
      </main>

      <AuthDialog
        open={authOpen}
        mode={authMode}
        onModeChange={setAuthMode}
        onClose={() => setAuthOpen(false)}
      />
      <CreateAuctionDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
      />
    </div>
  );
}

function Header({
  onOpenAuth,
  onCreate,
}: {
  onOpenAuth: (mode: "login" | "register") => void;
  onCreate: () => void;
}) {
  const { user, isAuthenticated } = useAuth();
  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-4 border-b bg-background/90 px-5 backdrop-blur supports-[backdrop-filter]:bg-background/70">
      <button
        onClick={() => navigate("#/")}
        className="flex items-center gap-2 font-display text-lg font-semibold tracking-tight"
      >
        <Gavel className="size-5 text-primary" />
        Pylon Auction House
      </button>
      <div className="flex-1" />
      <Button
        variant="outline"
        size="sm"
        onClick={onCreate}
        className="hidden sm:inline-flex"
      >
        <Gavel className="size-4" />
        Host an auction
      </Button>
      {isAuthenticated && user ? (
        <UserMenu user={user} />
      ) : (
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onOpenAuth("login")}
          >
            <UserIcon className="size-4" />
            Log in
          </Button>
          <Button size="sm" onClick={() => onOpenAuth("register")}>
            Sign up
          </Button>
        </div>
      )}
    </header>
  );
}

function SignInWall({ onPrompt }: { onPrompt: () => void }) {
  return (
    <main className="mx-auto flex max-w-md flex-col items-center gap-4 p-8 text-center">
      <ShieldCheck className="size-12 text-muted-foreground" />
      <h2 className="text-xl font-semibold">Sign in to view your bids</h2>
      <p className="text-sm text-muted-foreground">
        Watchlists, bid history, and balance live with your account.
      </p>
      <Button onClick={onPrompt}>Sign in</Button>
    </main>
  );
}
