/**
 * Pylon Studio config for the Auction House example.
 *
 * Demonstrates the full configuration surface:
 *   - Brand override (custom name + emoji glyph)
 *   - Theme accent (emerald, the default — kept here for clarity)
 *   - Hand-curated sidebar with sections, page items, resource items,
 *     a heading, and an external link
 *   - Per-resource column overrides with avatar / badge / date /
 *     currency / link renderers
 *   - Sample bulk + row actions (delete + export)
 *   - A `kind: "action"` row button that calls a server function against
 *     the clicked row and copies the result ("Copy link" on Auctions)
 *   - Custom column renderer ("paddle") that lives in studio.entry.tsx
 *   - Custom dashboard page ("liveBids") also from studio.entry.tsx
 *   - Used-space style footer card
 *
 * Run `bun run studio.config.ts` to verify the file evaluates cleanly.
 * The CLI does this automatically on `pylon dev`.
 */
import { defineStudioConfig } from "@pylonsync/sdk";

const config = defineStudioConfig({
  brand: {
    name: "Auction House",
    logo: "🛎️",
    subtitle: "Live bidding studio",
  },
  theme: {
    accent: "emerald",
    appearance: "dark",
  },
  sidebar: {
    orgSwitcher: {
      items: [
        { id: "default", label: "Auction House" },
        { id: "stage", label: "Staging" },
      ],
    },
    sections: [
      {
        label: "DASHBOARD",
        items: [
          { type: "page", id: "overview", label: "Overview", icon: "grid" },
          {
            type: "page",
            id: "liveBids",
            label: "Live bid feed",
            icon: "radio",
          },
        ],
      },
      {
        label: "AUCTIONS",
        items: [
          {
            type: "resource",
            entity: "Auction",
            label: "Auctions",
            icon: "calendar",
          },
          { type: "resource", entity: "Lot", label: "Lots", icon: "package" },
          { type: "resource", entity: "Bid", label: "Bids", icon: "tag" },
          {
            type: "resource",
            entity: "Watch",
            label: "Watchlist",
            icon: "star",
          },
        ],
      },
      {
        label: "PEOPLE",
        items: [
          { type: "resource", entity: "User", label: "Users", icon: "users" },
          {
            type: "page",
            id: "roles",
            label: "Roles & Permissions",
            icon: "shield",
          },
        ],
      },
      {
        label: "ACCOUNTS",
        items: [
          { type: "heading", label: "Integrations" },
          {
            type: "link",
            label: "Stripe",
            href: "https://dashboard.stripe.com",
            icon: "receipt",
          },
          {
            type: "link",
            label: "Google Analytics",
            href: "https://analytics.google.com",
            icon: "bar-chart",
          },
        ],
      },
    ],
    footer: {
      type: "card",
      title: "Used space",
      description: "Your team has used 80% of available capacity.",
      progress: 0.8,
      action: { label: "Upgrade plan", href: "https://pylon.dev/pricing" },
    },
  },
  resources: {
    User: {
      label: "User",
      pluralLabel: "Users",
      icon: "users",
      list: {
        searchable: true,
        filterable: true,
        defaultSort: { field: "createdAt", order: "desc" },
        defaultPageSize: 10,
        bulkActions: [
          { id: "delete", label: "Delete selected", kind: "delete", confirm: "Delete selected users?" },
          { id: "export", label: "Export as JSON", kind: "export" },
        ],
        columns: [
          {
            field: "displayName",
            label: "Name",
            renderer: {
              kind: "avatar",
              imageField: "avatarColor",
              subtitleField: "email",
            },
            sortable: true,
            searchable: true,
          },
          {
            field: "email",
            label: "Email",
            renderer: { kind: "link", href: "mailto:{value}" },
            searchable: true,
            hidden: true, // already shown as the avatar subtitle
          },
          {
            field: "balanceCents",
            label: "Balance",
            renderer: { kind: "number", style: "currency", currency: "USD" },
            align: "right",
            sortable: true,
          },
          {
            field: "createdAt",
            label: "Joined",
            renderer: { kind: "date", format: "relative" },
            sortable: true,
          },
        ],
      },
    },
    Auction: {
      label: "Auction",
      pluralLabel: "Auctions",
      icon: "calendar",
      list: {
        searchable: true,
        filterable: true,
        defaultSort: { field: "startsAt", order: "desc" },
        columns: [
          { field: "title", label: "Title", searchable: true, sortable: true },
          {
            field: "kind",
            label: "Type",
            renderer: {
              kind: "badge",
              variants: { live: "blue", timed: "gray" },
            },
            filterable: {
              options: [
                { label: "Live", value: "live" },
                { label: "Timed", value: "timed" },
              ],
            },
          },
          {
            field: "status",
            label: "Status",
            renderer: {
              kind: "badge",
              variants: {
                running: "green",
                scheduled: "blue",
                ended: "gray",
                draft: "amber",
              },
            },
            filterable: {
              options: [
                { label: "Running", value: "running" },
                { label: "Scheduled", value: "scheduled" },
                { label: "Ended", value: "ended" },
                { label: "Draft", value: "draft" },
              ],
            },
          },
          {
            field: "startsAt",
            label: "Starts",
            renderer: { kind: "date", format: "absolute" },
            sortable: true,
          },
          {
            field: "endsAt",
            label: "Ends",
            renderer: { kind: "date", format: "relative" },
            sortable: true,
          },
        ],
        rowActions: [
          // A server function run against one row, rendered as a button
          // in the row itself. `{row.id}` is substituted from the row
          // that was clicked; the returned `url` goes to the clipboard.
          {
            id: "shareLink",
            label: "Copy link",
            icon: "link",
            kind: "action",
            display: "button",
            action: "auctionShareLink",
            input: { auctionId: "{row.id}" },
            result: "copy",
            resultField: "url",
            refresh: false,
          },
          { id: "view", label: "Open", kind: "view" },
          {
            id: "delete",
            label: "Delete",
            kind: "delete",
            confirm: "Delete this auction?",
          },
        ],
      },
    },
    Lot: {
      label: "Lot",
      pluralLabel: "Lots",
      icon: "package",
      list: {
        searchable: true,
        defaultSort: { field: "position", order: "asc" },
        columns: [
          { field: "title", label: "Title", searchable: true, sortable: true },
          {
            field: "status",
            label: "Status",
            renderer: {
              kind: "badge",
              variants: {
                running: "green",
                pending: "gray",
                sold: "blue",
                passed: "amber",
              },
            },
          },
          {
            field: "currentCents",
            label: "Current bid",
            renderer: { kind: "number", style: "currency", currency: "USD" },
            align: "right",
            sortable: true,
          },
          {
            field: "bidCount",
            label: "Bids",
            renderer: { kind: "number" },
            align: "right",
          },
          // Custom renderer — the actual component lives in studio.entry.tsx
          {
            field: "auctionId",
            label: "Paddle",
            renderer: { kind: "custom", componentId: "paddle" },
          },
        ],
      },
    },
    Bid: {
      list: {
        defaultSort: { field: "createdAt", order: "desc" },
        defaultPageSize: 25,
        columns: [
          { field: "bidderName", label: "Bidder", searchable: true },
          {
            field: "amountCents",
            label: "Amount",
            renderer: { kind: "number", style: "currency", currency: "USD" },
            align: "right",
            sortable: true,
          },
          {
            field: "createdAt",
            label: "When",
            renderer: { kind: "date", format: "relative" },
            sortable: true,
          },
          { field: "lotId", label: "Lot", renderer: { kind: "text", mono: true } },
        ],
      },
    },
    Watch: {
      label: "Watch",
      pluralLabel: "Watchlist",
      icon: "star",
      hidden: false,
    },
  },
  pages: {
    liveBids: {
      subtitle: "Streaming new bids in real time.",
      componentId: "liveBids",
    },
  },
});

export default config;
