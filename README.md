# pylon

A self-hostable, full-stack framework for web, mobile, and real-time apps.

[![CI](https://github.com/pylonsync/pylon/actions/workflows/ci.yml/badge.svg)](https://github.com/pylonsync/pylon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![skills.sh](https://skills.sh/b/pylonsync/pylon)](https://skills.sh/pylonsync/pylon)

pylon gives you what Convex / Firebase / Supabase do — declarative schema,
real-time sync, server functions, auth, file storage — but as a Rust server
you self-host on a small VPS (SQLite or Postgres), not a stack of services.

```sh
# Install the pylon binary
curl -fsSL https://www.pylonsync.com/install.sh | bash

# Scaffold a full-stack app — auth, a multi-tenant dashboard, and
# server-rendered React, all on one port
npm create @pylonsync/pylon@latest my-app
cd my-app

# Dev server with live reload
pylon dev
```

Open `http://localhost:4321`. Prefer just the backend? `pylon init my-app`
scaffolds an API-only project (add a frontend with `--frontend
react|tanstack|nextjs`), and `http://localhost:4321/studio` is the inspector.

## Use with your coding agent

Pylon ships a [skill](skills/pylon/SKILL.md) that teaches Claude Code, Codex,
or Cursor how to build Pylon apps correctly — schema, policies, server
functions, SSR, and the gotchas. Add it to your agent with one command:

```sh
npx skills add pylonsync/pylon
```

`npm create @pylonsync/pylon@latest` also offers to install it during scaffold.

## What you get

- **Declarative schema** in JSON or DSL → tables, types, OpenAPI, client types
- **Real-time sync** — clients see updates as they happen (WebSocket + SSE)
- **TypeScript functions** — `mutation`/`query`/`action` with typed `ctx.db`
  - Handler IS the transaction (atomic by default)
  - Streaming responses for AI chat / live data
- **Auth** — sessions, magic codes, OAuth (Google + GitHub), RBAC
- **SSR** — file-based React routes, `<Link>` with instant client nav, `<Image>`
  with built-in Rust optimizer (mozjpeg + libwebp), Tailwind v4 first-class —
  full-stack apps without Next.js. See [examples/ssr-hello](examples/ssr-hello/)
  and the [SSR docs](https://docs.pylonsync.com/ssr/overview).
- **Background jobs** + cron scheduler
- **File storage**, **email**, **rate limiting**, **policies**, **plugins**

## How does it compare?

|  | pylon | Convex | Supabase | Firebase |
|---|---|---|---|---|
| Self-host | ✅ binary + Bun | ✅ docker-compose | ✅ multi-service | ❌ |
| Deploy targets | managed cloud, self-host, AWS, Workers (experimental) | their cloud or self-host | their cloud, self-host, k8s | their cloud only |
| Real-time sync | ✅ | ✅ reactive | ✅ Realtime | ✅ |
| Server functions | ✅ TypeScript | ✅ TypeScript | ✅ Edge Functions (Deno) | ✅ Cloud Functions |
| Native SSR | ✅ file-based React, one port | ❌ | ❌ | ❌ |
| Built on | Rust + SQLite | Rust + custom db | PG + Go + Deno | proprietary |
| One service, one port | ✅ | ❌ | ❌ | n/a |

## Quickstart

Install the binary (Bun ≥ 1.0 is also needed at runtime — Pylon runs your TypeScript and SSR on it):

```sh
curl -fsSL https://www.pylonsync.com/install.sh | bash
# or: cargo install pylon-cli   •   docker pull ghcr.io/pylonsync/pylon:latest
```

The fastest start is `npm create @pylonsync/pylon@latest my-app` (above) — a full-stack app you run with `npm run dev`, no global install needed. To build a backend by hand, one `app.ts` is the whole thing — schema + policies:

```ts
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

const Todo = entity("Todo", {
  title: field.string(),
  done: field.boolean().default(false),
  authorId: field.string(),
});

const todoPolicy = policy({
  name: "todo_owner",
  entity: "Todo",
  allowRead: "true",
  allowInsert: "auth.userId == data.authorId",
  allowUpdate: "auth.userId == data.authorId",
  allowDelete: "auth.userId == data.authorId",
});

const manifest = buildManifest({
  name: "todos",
  version: "0.1.0",
  entities: [Todo],
  policies: [todoPolicy],
});

console.log(JSON.stringify(manifest, null, 2)); // pylon dev reads stdout as the manifest
export default manifest;
```

Add a server function under `functions/createTodo.ts`:

```ts
import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { title: v.string() },
  async handler(ctx, args) {
    return {
      id: await ctx.db.insert("Todo", {
        title: args.title,
        done: false,
        authorId: ctx.auth.userId,
      }),
    };
  },
});
```

`pylon dev` serves it on `http://localhost:4321`. Read it from React with the typed client:

```tsx
import { db, callFn } from "@pylonsync/react";

function TodoList() {
  const { data: todos } = db.useQuery("Todo"); // live — updates as rows change
  return (
    <>
      {todos.map((t) => <li key={t.id}>{t.title}</li>)}
      <button onClick={() => callFn("createTodo", { title: "New todo" })}>Add</button>
    </>
  );
}
```

Full walkthrough — auth, live sync, and deploy — in the [Quickstart docs](https://docs.pylonsync.com/quickstart).

## Project layout

```
pylon/
├── crates/
│   ├── core/            Shared types, error codes, utilities
│   ├── http/            Platform-agnostic HTTP types + DataStore trait
│   ├── runtime/         SQLite-backed dev/prod server
│   ├── router/          HTTP routing logic, reused across platforms
│   ├── workers/         Cloudflare Workers adapter (experimental)
│   ├── functions/       Rust side of the TypeScript function runtime
│   ├── realtime/        Sharded game/collab server primitives
│   ├── auth/            Sessions, magic codes, OAuth, RBAC
│   ├── policy/          Access control rules engine
│   ├── sync/            Change log + push/pull
│   ├── storage/         SQLite + Postgres backends, file storage
│   ├── plugin/          Built-in plugins (cache, webhooks, soft delete, ...)
│   ├── migrate/         Schema migration diff engine
│   ├── cli/             The `pylon` binary
│   └── ...
└── packages/
    ├── sdk/             Schema DSL + manifest builder
    ├── react/           React hooks + typed client
    ├── react-native/    RN hooks + offline storage
    ├── next/            Next.js integration
    ├── swift/           Swift SDK (iOS, macOS, Linux) — sync, realtime, SwiftUI
    ├── functions/       Function definitions + Bun runtime
    ├── sync/            Sync engine (optimistic + offline-capable)
    ├── workflows/       Durable workflow runner
    └── create-pylon/  Project scaffolder
```

## Configuration

All configuration is via environment variables. See `crates/runtime/src/config.rs`.

Common settings:

```sh
PYLON_PORT=4321
PYLON_DB_PATH=/var/lib/pylon/pylon.db
PYLON_FILES_DIR=/var/lib/pylon/uploads
PYLON_SESSION_DB=/var/lib/pylon/sessions.db
PYLON_ADMIN_TOKEN=<long random>             # operator-role bearer
PYLON_ADMIN_EMAILS=ops@your-domain.com      # human admins (verified email allowlist)
PYLON_CORS_ORIGIN=https://your-app.com
PYLON_DEV_MODE=false
```

## Deployment

- **Managed cloud**: [pylonsync.com](https://www.pylonsync.com) — same binary, hosted; sign up and `pylon deploy`
- **Self-host**: `curl … | bash` or `docker run` — see [docs/ops/DEPLOY.md](docs/ops/DEPLOY.md)
- **AWS ECS**: see `deploy/terraform/` and `deploy/sst/`
- **Cloudflare Workers**: see `crates/workers/README.md` (experimental)

Architecture docs:
- [RUNTIME.md](docs/RUNTIME.md) — how TypeScript functions execute, what JS engine, what we evaluated
- [SYNC.md](docs/SYNC.md) — sync semantics, CRDT-backed rows, offline behavior
- [ARCHITECTURE.md](ARCHITECTURE.md) — crate-by-crate map of the system

Operational docs:
- [DEPLOY.md](docs/ops/DEPLOY.md) — env vars, reverse proxy, health checks
- [SIZING.md](docs/ops/SIZING.md) — measured throughput, capacity planning
- [TOKEN_ROTATION.md](docs/ops/TOKEN_ROTATION.md) — admin token rotation
- [INCIDENT.md](docs/ops/INCIDENT.md) — incident response playbook
- [WORKERS_COSTS.md](docs/ops/WORKERS_COSTS.md) — cost patterns on Cloudflare

## Project status

**Pre-1.0.** API is stable enough to build with but may evolve. SQLite is
the default backend and Postgres is supported for deployments that need an
external database or horizontal database operations. Cloudflare Workers / D1
is experimental — see `SECURITY.md` for a list of pre-1.0 hardening gaps.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and PRs welcome.

## Security

See [SECURITY.md](SECURITY.md) for vulnerability reporting and hardening
notes. **Do not file security issues publicly.** Email security@pylonsync.com.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option.
