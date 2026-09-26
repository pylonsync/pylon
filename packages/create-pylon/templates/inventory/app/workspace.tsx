"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Package } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";
import {
  onHandByProduct,
  summarize,
  type Movement,
  type Product,
  type Summary,
} from "@/lib/stock";

export type ProductRow = Product;
export type MovementRow = Movement;
export interface UserRow {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface WorkspaceData {
  products: ProductRow[];
  movements: MovementRow[];
  /** On-hand for every product, computed once per render rather than per row. */
  levels: Map<string, number>;
  productName: (id: string | null | undefined) => string | null;
  actorName: (id: string | null | undefined) => string | null;
  /** Totals from the ledger: SKUs, units, value, low and out counts. */
  summary: Summary;
  /**
   * The number beside each nav link, keyed by href. Page headers and tabs
   * read the same summary, so the sidebar and the page can't disagree.
   */
  navCounts: Record<string, number>;
  loading: boolean;
}

/**
 * The app shell, and the ONLY place that touches `db`.
 *
 * Every view below receives plain data through the render prop and reports
 * changes through callbacks, which is what lets the components in `components/`
 * render from fixtures in a test with nothing mocked.
 *
 * The queries are live: a movement recorded at the back door updates the level
 * on the shop floor\'s screen immediately, so nobody sells what was just
 * damaged.
 */
export function Workspace({
  pathname,
  children,
}: {
  pathname: string;
  children: (data: WorkspaceData) => React.ReactNode;
}) {
  const router = useRouter();
  // The signed-in identity comes from the CLIENT session, not from an SSR
  // prop: the session cookie is SameSite=Lax and browsers withhold it inside
  // the builder's cross-site preview iframe, so a server-resolved email would
  // be empty exactly where this app is most often viewed.
  const { signOut, userId } = useAuth();
  const { data: products, loading } = db.useQuery<ProductRow>("Product");
  const { data: movements } = db.useQuery<MovementRow>("Movement");
  const { data: users } = db.useQuery<UserRow>("User");

  // Whoever is signed in, named from the synced User row rather than an
  // SSR prop — see the note on useAuth above.
  const me = (users ?? []).find((user) => user.id === userId);

  const [paletteOpen, setPaletteOpen] = useState(false);
  const seeded = useRef(false);

  const productList = products ?? [];
  const movementList = movements ?? [];

  useEffect(() => {
    if (loading || seeded.current) return;
    if (productList.length > 0) {
      seeded.current = true;
      return;
    }
    seeded.current = true;
    callFn("seedWorkspace", {}).catch(() => {
      seeded.current = false;
    });
  }, [loading, productList.length]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if (event.key === "/" && !typing) {
        event.preventDefault();
        setPaletteOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // One pass over the ledger for the whole render — calling onHand() per row
  // would be O(products x movements).
  const levels = useMemo(() => onHandByProduct(movementList), [movementList]);
  const productById = useMemo(
    () => new Map(productList.map((p) => [p.id, p] as const)),
    [productList],
  );
  const userById = useMemo(
    () => new Map((users ?? []).map((u) => [u.id, u] as const)),
    [users],
  );

  const searchIndex = useMemo<SearchItem[]>(
    () =>
      productList.map((product) => ({
        id: `product:${product.id}`,
        type: "product",
        title: product.name,
        subtitle: product.sku,
        href: `/products/${product.id}`,
        keywords: product.category ?? undefined,
      })),
    [productList],
  );

  const summary = summarize(productList, movementList);
  const navCounts = { "/": summary.skuCount, "/movements": movementList.length };

  const data: WorkspaceData = {
    products: productList,
    movements: movementList,
    levels,
    productName: (id) => productById.get(id ?? "")?.name ?? null,
    actorName: (id) => {
      const user = userById.get(id ?? "");
      return user?.displayName || user?.email || null;
    },
    summary,
    navCounts,
    loading,
  };

  return (
    <AppShell
      sidebar={{
        user: { name: me?.displayName ?? "", email: me?.email ?? "" },
        pathname,
        counts: navCounts,
        onOpenCommand: () => setPaletteOpen(true),
        onSignOut: () => void signOut(),
      }}
    >
      {children(data)}

      <CommandPalette
        open={paletteOpen}
        items={searchIndex}
        actions={[
          {
            id: "new-product",
            label: "New product",
            run: () => router.push("/?new=product"),
          },
          {
            id: "reorder",
            label: `Needs reorder (${summary.lowCount + summary.outCount})`,
            run: () => router.push("/?filter=reorder"),
          },
          {
            id: "go-movements",
            label: "Go to Movements",
            run: () => router.push("/movements"),
          },
        ]}
        placeholder="Search products by name, SKU, or category…"
        icons={{ product: <Package /> }}
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </AppShell>
  );
}
