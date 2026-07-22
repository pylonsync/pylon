# pylon

A self-hostable, full-stack framework for web, mobile, and real-time apps.

[![CI](https://github.com/pylonsync/pylon/actions/workflows/ci.yml/badge.svg)](https://github.com/pylonsync/pylon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![skills.sh](https://skills.sh/b/pylonsync/pylon)](https://skills.sh/pylonsync/pylon)

Like Convex, Firebase, and Supabase, pylon gives you declarative schema,
real-time sync, server functions, auth, and file storage. It packages them in a
Rust server you can self-host on a small VPS with SQLite or Postgres.

```sh
# Scaffold a full-stack app with auth, a multi-tenant dashboard, and
# server-rendered React, all on one port. No global install needed.
npm create @pylonsync/pylon@latest my-app
cd my-app
npm run dev
```

Open `http://localhost:4321`. The Pylon CLI ships as a dependency, so `npm run
dev` runs it. You only need [Bun](https://bun.sh) ≥ 1.0 on your PATH (Pylon runs
your TypeScript and SSR on it).

Want `pylon` on your PATH (for `pylon init`, or `pylon deploy` from anywhere)?
`curl -fsSL https://www.pylonsync.com/install.sh | bash`. Prefer a backend-only
project? `pylon init my-app` scaffolds an API-only app (add a frontend with
`--frontend react|tanstack|nextjs`); `http://localhost:4321/studio` is the
inspector.

## Use with your coding agent

Pylon includes a [skill](skills/pylon/SKILL.md) with guidance for Claude Code,
Codex, and Cursor on schema, policies, server functions, SSR, and common
gotchas. Add it to your agent with one command:

```sh
npx skills add pylonsync/pylon
```

`npm create @pylonsync/pylon@latest` also offers to install it during scaffold.

## What you get

- **Declarative schema** in JSON or DSL → tables, types, OpenAPI, client types
- **Real-time sync** over WebSocket and SSE
- **TypeScript functions**: `query`/`mutation`/`action` with a typed `ctx`
  - The mutation handler IS the transaction (atomic by default)
  - Streaming responses for AI chat / live data
- **Auth**: sessions, passwords, magic codes, OAuth (25 providers), passkeys,
  TOTP, RBAC, organizations
- **SSR**: file-based React routes, `<Link>` with instant client nav, `<Image>`
  with a built-in Rust optimizer (AVIF/WebP/JPEG), and first-class Tailwind v4;
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
| Built on | Rust + Bun + SQLite | Rust + custom db | PG + Go + Deno | proprietary |
| One service, one port | ✅ | ❌ | ❌ | n/a |

## Quickstart

`npm create @pylonsync/pylon@latest` is the fastest way to start a full-stack
app, with no global install. To build a **backend by hand**, install the `pylon`
binary. Bun ≥ 1.0 is also required at runtime for TypeScript and SSR:

```sh
curl -fsSL https://www.pylonsync.com/install.sh | bash
# or: cargo install --git https://github.com/pylonsync/pylon pylon-cli
# or: docker pull ghcr.io/pylonsync/pylon:latest
```

The schema and policies for this backend fit in one `app.ts`:

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
  const { data: todos } = db.useQuery("Todo"); // updates live as rows change
  return (
    <>
      {todos.map((t) => <li key={t.id}>{t.title}</li>)}
      <button onClick={() => callFn("createTodo", { title: "New todo" })}>Add</button>
    </>
  );
}
```

The [Quickstart docs](https://docs.pylonsync.com/quickstart) cover auth, live
sync, and deployment.

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
│   ├── plugin/          Built-in plugins (cache, rate limit, CSRF, tenant scoping, ...)
│   ├── migrate/         Schema migration diff engine
│   ├── cli/             The `pylon` binary
│   └── ...
└── packages/
    ├── sdk/             Schema DSL + manifest builder
    ├── react/           React hooks + typed client
    ├── react-native/    RN hooks + offline storage
    ├── next/            Next.js integration
    ├── swift/           Swift SDK (iOS, macOS, Linux): sync, realtime, SwiftUI
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

- **Managed cloud**: [pylonsync.com](https://www.pylonsync.com) hosts the same
  binary; sign up and run `pylon deploy`
- **Self-host**: use `curl … | bash` or `docker run`; see
  [docs/ops/DEPLOY.md](docs/ops/DEPLOY.md)
- **AWS ECS**: see `deploy/terraform/` and `deploy/sst/`
- **Cloudflare Workers**: see `crates/workers/README.md` (experimental)

Architecture docs:
- [RUNTIME.md](docs/RUNTIME.md): how TypeScript functions execute, which JS
  engine Pylon uses, and the alternatives evaluated
- [SYNC.md](docs/SYNC.md): sync semantics, CRDT-backed rows, and offline behavior
- [ARCHITECTURE.md](ARCHITECTURE.md): crate-by-crate map of the system

Operational docs:
- [DEPLOY.md](docs/ops/DEPLOY.md): env vars, reverse proxy, and health checks
- [SIZING.md](docs/ops/SIZING.md): measured throughput and capacity planning
- [TOKEN_ROTATION.md](docs/ops/TOKEN_ROTATION.md): admin token rotation
- [INCIDENT.md](docs/ops/INCIDENT.md): incident response playbook
- [WORKERS_COSTS.md](docs/ops/WORKERS_COSTS.md): cost patterns on Cloudflare

## Project status

**Pre-1.0.** API is stable enough to build with but may evolve. SQLite is
the default backend and Postgres is supported for deployments that need an
external database or horizontal database operations. Cloudflare Workers / D1
is experimental. See `SECURITY.md` for a list of pre-1.0 hardening gaps.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and PRs welcome.

## Security

See [SECURITY.md](SECURITY.md) for vulnerability reporting and hardening
notes. **Do not file security issues publicly.** Email security@pylonsync.com.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
at your option.
