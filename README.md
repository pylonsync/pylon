# pylon

A self-hostable, single-binary backend for web, mobile, and real-time apps.

[![CI](https://github.com/pylonsync/pylon/actions/workflows/ci.yml/badge.svg)](https://github.com/pylonsync/pylon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

pylon gives you what Convex / Firebase / Supabase do — declarative schema,
real-time sync, server functions, auth, file storage — but as a single Rust
binary you can `scp` to a VPS or run with SQLite or Postgres.

```sh
# Install
curl -fsSL https://pylonsync.com/install.sh | bash

# New project
pylon init my-app
cd my-app

# Dev server with live reload
pylon dev
```

Visit `http://localhost:4321/studio` for the inspector.

## What you get

- **Declarative schema** in JSON or DSL → tables, types, OpenAPI, client types
- **Real-time sync** — clients see updates as they happen (WebSocket + SSE)
- **TypeScript functions** — `mutation`/`query`/`action` with typed `ctx.db`
  - Handler IS the transaction (atomic by default)
  - Streaming responses for AI chat / live data
- **Auth** — sessions, magic codes, OAuth (Google + GitHub), RBAC
- **Real-time shards** for multiplayer games & collab apps
  - Tick-driven simulations
  - Matchmaker, area-of-interest, replay
- **Background jobs** + cron scheduler
- **Workflows** — long-running, durable
- **File storage**, **email**, **rate limiting**, **policies**, **plugins**

## How does it compare?

|  | pylon | Convex | Supabase | Firebase |
|---|---|---|---|---|
| Self-host | ✅ single binary | ✅ docker-compose | ✅ multi-service | ❌ |
| Deploy targets | self-host, AWS, Workers (experimental) | their cloud or self-host | their cloud, self-host, k8s | their cloud only |
| Real-time sync | ✅ | ✅ reactive | ✅ Realtime | ✅ |
| Server functions | ✅ TypeScript | ✅ TypeScript | ✅ Edge Functions (Deno) | ✅ Cloud Functions |
| Game shards | ✅ tick-based | ❌ | ❌ | ❌ |
| Built on | Rust + SQLite | Rust + custom db | PG + Go + Deno | proprietary |
| Single process | ✅ | ❌ | ❌ | n/a |

## Quickstart

### 1. Install

```sh
curl -fsSL https://pylonsync.com/install.sh | bash
```

Other install paths:

```sh
# Homebrew (macOS + Linux)
brew install pylonsync/tap/pylon

# Cargo (compiles from source)
cargo install pylon-cli

# Docker
docker pull ghcr.io/pylonsync/pylon:latest
```

### 2. Define your schema

`pylon.manifest.json`:

```json
{
  "manifest_version": 1,
  "name": "todos",
  "version": "0.1.0",
  "entities": [
    { "name": "Todo", "fields": [
      { "name": "title", "type": "string", "optional": false, "unique": false },
      { "name": "done", "type": "bool", "optional": false, "unique": false }
    ], "indexes": [], "relations": [] }
  ],
  "routes": [], "queries": [], "actions": [], "policies": []
}
```

### 3. Run

```sh
pylon dev
```

### 4. Connect from React

```tsx
import { init, db } from "@pylon/react";
init({ baseUrl: "http://localhost:4321" });

function TodoList() {
  const { data: todos } = db.useQuery("Todo");
  const { mutate: add } = db.useMutation("createTodo");

  return (
    <>
      {todos.map(t => <li key={t.id}>{t.title}</li>)}
      <button onClick={() => add({ title: "New todo" })}>Add</button>
    </>
  );
}
```

### 5. Add server-side logic

`functions/createTodo.ts`:

```ts
import { mutation, v } from "@pylon/functions";

export default mutation({
  args: { title: v.string() },
  async handler(ctx, args) {
    const id = await ctx.db.insert("Todo", {
      title: args.title,
      done: false,
      authorId: ctx.auth.userId,
    });
    return { id };
  },
});
```

### 6. Add a multiplayer shard (optional)

```rust
use pylon_realtime::{Shard, ShardConfig, SimState};

struct MyGame { /* state */ }
impl SimState for MyGame { /* tick, snapshot, apply_input */ }

let shard = Shard::new("match_1", MyGame::default(), ShardConfig {
    tick_rate_hz: 20,
    ..Default::default()
});
```

Then connect from the client:

```tsx
import { useShard } from "@pylon/react";
const { snapshot, send } = useShard("match_1", { subscriberId: "player_42" });
```

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
PYLON_ADMIN_TOKEN=<long random>
PYLON_CORS_ORIGIN=https://your-app.com
PYLON_DEV_MODE=false
```

## Deployment

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
