# Pylon Studio

Pylon Studio is the admin dashboard the framework serves at `/studio` for
every Pylon app. It introspects your manifest, exposes live data tables
with pagination/sorting/filtering, and lets you re-skin the whole
experience to match your product.

## What you get out of the box

- Auto-derived sidebar — every entity in the manifest becomes a left-nav
  resource. No config needed.
- Built-in dark theme with an emerald accent (matches the screenshot in
  the README).
- Pages for the framework internals: Manifest, Functions, Policies,
  Routes, Live sync, Health, Settings, Roles & Permissions.
- A `Locked` view for unauthenticated users — the data layer never leaks
  rows to anonymous callers.

That is what the Studio looks like with no project changes. The rest of
this document is about *customizing* it.

## studio.config.ts

Drop a `studio.config.ts` next to your `app.ts`:

```ts
import { defineStudioConfig } from "@pylonsync/sdk";

export default defineStudioConfig({
  brand: {
    name: "Acme",
    logo: "🛎️",         // emoji glyph, image URL, or data URL
    subtitle: "v1.0",
  },
  theme: {
    accent: "emerald",   // emerald | blue | violet | rose | amber
    appearance: "dark",  // dark | light | system
    primary: "#10b981",  // optional custom override
  },
  sidebar: {
    sections: [
      {
        label: "DASHBOARD",
        items: [
          { type: "page", id: "overview", label: "Overview", icon: "grid" },
        ],
      },
      {
        label: "RESOURCES",
        items: [
          { type: "resource", entity: "User", icon: "users" },
          { type: "resource", entity: "Post", icon: "file-text" },
        ],
      },
      {
        label: "ACCOUNTS",
        items: [
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
      action: { label: "Upgrade", href: "https://example.com/billing" },
    },
  },
  resources: {
    User: {
      list: {
        searchable: true,
        defaultSort: { field: "createdAt", order: "desc" },
        bulkActions: [{ id: "delete", label: "Delete", kind: "delete" }],
        columns: [
          {
            field: "name",
            label: "Name",
            renderer: {
              kind: "avatar",
              imageField: "avatar",
              subtitleField: "email",
            },
            sortable: true,
            searchable: true,
          },
          {
            field: "role",
            label: "Role",
            sortable: true,
            filterable: {
              options: [
                { label: "Admin", value: "admin" },
                { label: "User", value: "user" },
              ],
            },
          },
          {
            field: "status",
            label: "Status",
            renderer: {
              kind: "badge",
              variants: { active: "green", blocked: "red" },
            },
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
  },
});
```

The CLI runs `bun run studio.config.ts` on `pylon dev` (and `pylon
start`), validates the result against the kernel struct, and writes
`.pylon/studio.config.json`. The runtime serves it inline with the
Studio HTML. Edits to `studio.config.ts` take effect on refresh — no
server restart.

## Cell renderers

Each column has an optional `renderer`. Built-ins:

| `kind`     | Notes                                                                       |
| ---------- | --------------------------------------------------------------------------- |
| `text`     | Default. `truncate`, `mono` options.                                        |
| `avatar`   | Initials + optional `imageField` + `subtitleField`. Matches the screenshot. |
| `badge`    | Colored pill with optional dot. `variants` maps value → color.              |
| `date`     | `relative` (default) / `absolute` / `iso`.                                  |
| `link`     | `href` template with `{value}` and `{row.field}` interpolation.             |
| `boolean`  | `trueLabel` / `falseLabel`.                                                 |
| `number`   | `decimal` / `percent` / `currency` (with `currency` ISO code) / `bytes`.    |
| `json`     | Compact preview, full value in `title`.                                     |
| `custom`   | Resolved against the extensions registry — see below.                       |

## studio.entry.tsx — custom React components

If you need anything beyond the built-in renderers (a pricing chart in a
cell, a custom dashboard page, a footer slot with tenant-specific
billing info), drop a `studio.entry.tsx`:

```tsx
import { defineStudioExtensions } from "@pylonsync/sdk";

function PaddleCell({ value, row }) {
  return <code>#{hash(value, row.id)}</code>;
}

function MyDashboard({ api, manifest }) {
  // ...
  return <div>...</div>;
}

export default defineStudioExtensions({
  renderers: { paddle: PaddleCell },
  pages: { dashboard: MyDashboard },
});
```

Reference them from `studio.config.ts` by id:

```ts
{ field: "lotId", renderer: { kind: "custom", componentId: "paddle" } }
{ type: "page", id: "dashboard", label: "Dashboard", icon: "grid" }
```

The CLI bundles this file with `bun build` — `react`, `react-dom`, and
`react/jsx-runtime` are externalized, so the bundle is small. Output
lands at `.pylon/studio.entry.js`; the runtime serves it at
`/studio/extensions.js`.

## Server-side list params

`/api/entities/<Entity>` accepts:

- `?page=N&per_page=M` — paginated envelope `{data, total, page, per_page}`
- `?sort=field&order=asc|desc`
- `?q=...` — text search (uses FTS5 on SQLite when the entity declares
  `search.text`)
- `?filter[field]=value` — per-field equality filter

Without `page`, the legacy envelope `{data, count, offset, limit}` is
returned for backwards compatibility — old clients keep working
unchanged. The Studio web shell sends `page` + `per_page` always.

## Default theme accents

| Accent     | Use when                                        |
| ---------- | ----------------------------------------------- |
| `emerald`  | Default. Matches the Refine reference design.   |
| `blue`     | Standard SaaS feel.                             |
| `violet`   | Creative tools, design surfaces.                |
| `rose`     | Consumer apps, retail.                          |
| `amber`    | Finance, auctions, trading.                     |

`theme.primary` accepts any CSS color value to override the accent
entirely. Useful when matching a customer's brand exactly.

## Example

The `examples/auction-house` project ships a complete `studio.config.ts`
+ `studio.entry.tsx` pair demonstrating every feature in this README.

## File reference

```
your-app/
├── app.ts                   # manifest source
├── studio.config.ts         # (optional) studio configuration
├── studio.entry.tsx         # (optional) custom React components
└── .pylon/
    ├── dev.db               # local SQLite (dev only)
    ├── studio.config.json   # compiled config — do not edit
    └── studio.entry.js      # bundled extensions — do not edit
```
