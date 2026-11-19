# Pylon Studio

Studio is the admin UI that every Pylon app serves at `/studio`. An operator
uses it to read and change rows, call functions, and see the state of the
server: requests, jobs, workflows, sync events and shards.

The source is a Vite, React 19 and Tailwind CSS v4 app. The components in
`src/components/ui/` are built on [ui-sdk](https://ui-sdk.dev)
(`@ui-sdk/react` and `@ui-sdk/core`, pinned to one version).

## How it ships

`bun run build` writes one file, `dist/index.html`, with the script and the
styles inline. `crates/studio_api/src/lib.rs` embeds that file in the Pylon
binary with `include_str!` and fills three values for each response:

- `window.__PYLON_API__`: the origin of the API.
- `window.__PYLON_MANIFEST__`: the public manifest of the app.
- `window.__PYLON_STUDIO_CONFIG__`: the app's `studio.config.ts`, as JSON.

A change to the source reaches a running Pylon only after you build the
bundle and then build the binary.

The sign-in page, the "No operators yet" page and the "access denied" page are
not in this bundle. The server renders them as plain HTML in
`crates/runtime/src/server.rs` (`STUDIO_PAGE_HTML`).

## Build and check

```sh
cd crates/studio_api/web
bun install
bun run build   # type check, then write dist/index.html
bun test        # unit tests in src/lib
bun run lint
```

## Run it against a local app

1. Build the bundle (above), then build the CLI from the repository root:

   ```sh
   cargo build -p pylon-cli
   ```

2. In the directory of a Pylon app, start the dev server with that binary:

   ```sh
   /path/to/pylon/target/debug/pylon dev --port 4321
   ```

3. Create a Studio operator. The command asks for a password:

   ```sh
   /path/to/pylon/target/debug/pylon admin create <username>
   ```

   Add `--url http://localhost:<port>` when the port is not 4321. A server
   that runs with `PYLON_ADMIN_TOKEN` needs the same variable set for this
   command.

4. Open `http://localhost:4321/studio/login` and sign in.

## Layout of the source

| Path | Contents |
| --- | --- |
| `src/components/ui/` | Reusable parts on ui-sdk: button, input, select, menu, dialog, sheet, tabs, table, toast. No app data. |
| `src/components/` | Parts that know about Studio: the page frame, the row panel, the cell renderers, empty and error states. |
| `src/layout/` | The sidebar, the theme, and the nav groups. |
| `src/pages/` | One file for each page. `ResourceList.tsx` is the table for an entity. |
| `src/lib/` | The API client, the URL scheme, the `studio.config.ts` types, references between entities, formatting. |
| `src/hooks/` | `useQuery` (load and poll data), `useMetrics`, `useMediaQuery`, `useEscapeOutside`. |

## Where the runtime pages get their data

| Page | Endpoint |
| --- | --- |
| Overview, Metrics | `GET /metrics` with `Accept: application/json` |
| Health | `GET /admin/health`, plus `/metrics` for errors and latency |
| Logs: requests | `GET /admin/logs/tail?since=<timestamp>` (the last 1,000 requests, in memory) |
| Logs: function calls | `GET /admin/fn/traces` |
| Logs: audit events | `GET /api/admin/audit` |
| Logs: sessions | `GET /api/admin/auth/sessions` |
| Metrics: routes and functions | computed in the browser from the request log and the function traces |
| Jobs | `GET /api/jobs`, `/api/jobs/stats`, `/api/jobs/dead`, `POST /api/jobs/dead/<id>/retry`, `POST /api/jobs/<id>/cancel`, `GET /api/scheduler`, `POST /api/scheduler/trigger/<name>` |
| Workflows | `GET /api/workflows`, `/api/workflows/definitions`, `POST /api/workflows/<id>/cancel` |
| Live sync | `GET /admin/changes/stream` (Server-Sent Events) |
| System tables | `GET /api/admin/auth/<table>`, `GET /api/admin/ops/<table>` |

Every endpoint is admin only. A Studio operator session counts as an admin on
the requests that carry the `X-Pylon-Studio` header and on `/admin/*`.

## Extensions

An app can add components and action handlers with a `studio.entry.tsx`. The
CLI bundles it with `react` left as a bare import. The import map in
`index.html` points `react`, `react-dom`, `react-dom/client`,
`react/jsx-runtime` and `react/jsx-dev-runtime` at `/studio/modules/<file>.js`.
Each of those files re-exports the React that Studio put on
`globalThis.__PYLON_STUDIO_MODULES__` (`src/lib/shared-modules.ts`).
`scripts/build-shims.ts` writes them at the end of `bun run build`, with the
export names of the real modules, and the binary embeds them.

`src/lib/extensions.ts` holds the registry: `renderers`, `pages`, `layouts`
(`footer`, `headerActions`) and `actions`. An action handler runs for a
`kind: "custom"` row action or bulk action with the same id. The user guide is
`apps/docs/operations/studio.mdx`.

## Rules for changes

- Build a control on a ui-sdk part. Read the page of the part in
  `node_modules/@ui-sdk/react/docs/components/` first.
- Use the color tokens in `src/index.css`. Each token is one `light-dark()`
  pair, so a component needs no `dark:` class.
- Every data view has a loading state, an empty state and an error state
  (`src/components/states.tsx`).
- The types in `src/lib/studio-config.ts` mirror `packages/sdk/src/studio.ts`
  and `crates/core/src/studio.rs`. Change the three together.
