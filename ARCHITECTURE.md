# agentdb Architecture

## Product Definition

`agentdb` is an AI-native application runtime and framework for:

- web apps
- mobile apps
- static content sites
- local-first software with built-in sync

The framework should let AI tools build and operate apps through:

- code
- CLI commands
- machine-readable manifests
- structured diagnostics

The framework should not depend on dashboards or browser-based admin workflows for core functionality.

## Design Goals

- Single-binary server deployment
- Excellent local DX for both AI and humans
- One obvious way to model data, read data, write data, and define routes
- Local-first sync by default for app data
- Static rendering and live app rendering in the same framework
- First-class mobile support
- Predictable machine-readable control surface

## Non-Goals

- Rich CMS/editor product
- Dashboard-first backend management
- Multiple competing data APIs
- CRDT-heavy collaboration in v1
- Plugin marketplace in v1
- Multi-runtime product complexity in v1

## Core Thesis

The framework should center around a small set of primitives:

- `Schema`
- `Query`
- `Action`
- `Policy`
- `Route`
- `Asset`

Everything else should derive from these primitives.

## Language and Runtime Choices

### Core

- Rust for the runtime, sync engine, storage adapters, auth runtime, CLI, and deployable server

Why:

- single binary
- strong performance profile
- low memory footprint
- good cold starts
- better fit than Go for a sync/storage-heavy product

### App Surface

- TypeScript for app definitions and generated bindings
- React for web rendering
- React Native + Expo for mobile rendering

Why:

- AI already understands TS/JSX extremely well
- strong web and mobile ecosystem
- low friction for app authors

## Storage Model

### Local Dev

- SQLite by default
- optional local Postgres

### Production

- Postgres as canonical relational store
- SQLite as client/local replica
- object storage for blobs/assets

### Data Model

The server owns canonical state. Clients maintain local SQLite replicas for:

- offline access
- optimistic writes
- fast reads
- sync recovery

## App Model

An app is made of:

- typed schema
- typed queries
- typed actions
- declarative policies
- explicit routes
- views
- optional file-backed content
- optional data-backed content

## Filesystem Contract

User app layout:

```txt
/app
  /schema
    entities.ts
    relations.ts
    indexes.ts
    queries.ts
    actions.ts
    policies.ts
  /routes
    index.route.tsx
    blog-slug.route.tsx
    app-project-id.route.tsx
  /views
  /components
  /content
    docs/
    blog/
    pages/
  /styles
  /agents
    manifest.json
    context.md
    invariants.md
  /generated
/tests
agentdb.config.ts
package.json
```

Rules:

- `/generated` is framework-owned
- app code lives in `/app`
- all important behavior should be visible from files in the repo
- no dashboard-only configuration

## Core Primitives

### Schema

Defines:

- entities
- fields
- relations
- indexes
- content entities
- assets

Schema is the single source of truth for:

- storage
- generated client bindings
- query planning
- policies
- sync metadata

### Query

Typed graph reads that run in one of three modes:

- `static`
- `server`
- `live`

Queries are the only standard read path.

### Action

Typed transactional writes with:

- validation
- auth context
- policy checks
- idempotency
- undo metadata

Actions are the only standard write path.

### Policy

Declarative authorization rules at:

- entity level
- field level
- action level
- route access level

Policies run during query planning and mutation execution.

### Route

Every route must declare:

- path
- mode
- query
- view
- optional auth requirement
- optional SEO metadata

Route modes:

- `static`
- `server`
- `live`

### Asset

First-class file/blob records with:

- metadata
- ownership
- derived URLs
- storage references

## Content Model

There is no editor product in v1.

Content exists in two forms:

### File-Backed Content

For:

- docs
- blogs
- marketing pages
- changelogs

Stored in repo files. AI tools edit them through code or CLI workflows.

### Data-Backed Content

For:

- user-generated content
- dynamic pages
- app-managed content

Stored as normal entities and manipulated through actions.

The framework should support draft/publish states, but not a rich authoring UI.

## Sync Architecture

### Model

- server-authoritative
- client-local replica
- append-only change log
- cursor-based pull
- idempotent push
- optimistic local actions

### Client Responsibilities

- maintain local SQLite replica
- queue pending actions
- apply optimistic updates
- track sync cursor
- subscribe to invalidation hints

### Server Responsibilities

- validate auth and policies
- execute actions transactionally
- append durable change events
- serve pull updates since cursor
- dedupe repeated mutations

### Transport

Durable sync:

- HTTP push/pull

Realtime hints:

- WebSocket or SSE

Sockets should not be the source of truth. They only accelerate freshness.

### Conflict Policy

V1 defaults:

- entity versioning
- tombstones for deletes
- last-write-wins at field level
- structured mutation rejection payloads

Custom merge logic can be added later at the action level.

## Auth Model

V1 auth support:

- email link / magic code
- GitHub OAuth
- Google OAuth
- session cookies
- bearer tokens

Auth context must be available consistently in:

- routes
- queries
- actions
- policies

## Runtime Topology

### Default Deployment

- one Rust binary
- one Postgres database
- optional object storage

The binary should contain:

- HTTP server
- auth runtime
- query execution
- action execution
- sync endpoints
- static asset serving
- realtime invalidation transport

### Scaling Modes

#### Static

- prerendered output only
- host anywhere

#### Single-Binary

- default production mode
- one runtime process
- one Postgres

#### Horizontally Scaled

- many stateless runtime instances
- shared Postgres
- optional dedicated invalidation/sync fanout layer later

The architecture should allow scaling out without changing the programming model.

## Frontend Model

React still makes sense as the rendering layer, but not as the center of the framework.

Framework responsibilities:

- own data model
- own action model
- own route model
- own sync semantics
- own auth semantics

React responsibilities:

- render views
- bind to queries
- bind to actions

For static content:

- render HTML first
- hydrate only where needed

For app routes:

- use live queries and action bindings through generated hooks

## Mobile Model

Mobile support is first-class in v1.

Target:

- React Native + Expo

Requirements:

- local SQLite replica
- background-safe sync model
- generated TS bindings
- same schema/query/action surface as web

Web and mobile should share:

- schema
- actions
- queries
- route-adjacent view logic where practical
- generated client APIs

## CLI Contract

The CLI is the primary operator surface.

Every important command should support `--json`.

Initial command set:

- `agentdb init`
- `agentdb dev`
- `agentdb codegen`
- `agentdb studio`
- `agentdb schema check`
- `agentdb schema push`
- `agentdb query run <name>`
- `agentdb action run <name>`
- `agentdb explain <route|query|action|policy>`
- `agentdb sync inspect`
- `agentdb build`
- `agentdb deploy`
- `agentdb doctor`

### Most Important Commands

`agentdb dev`

- starts local runtime
- starts local storage
- watches schema, routes, views, content
- regenerates bindings
- exposes local Studio

`agentdb explain`

- explains framework interpretation of route/query/action/policy
- should be useful to both humans and AI

`agentdb doctor`

- validates environment
- validates schema
- validates auth configuration
- validates deploy readiness

## Introspection and Diagnostics

Every app should generate machine-readable metadata:

`/app/agents/manifest.json`

Should include:

- schema summary
- route inventory
- action signatures
- policy summary
- environment requirements
- deploy target
- test commands

Errors should be structured and machine-actionable:

- error code
- failing resource
- failing invariant
- probable fix

## Local Studio

Studio is a debug and inspection tool, not a CMS.

V1 Studio features:

- entity browser
- query runner
- action runner
- auth/session viewer
- policy evaluator
- sync inspector
- route manifest viewer
- schema graph viewer

No rich editor. No publishing workspace. No media management product.

## Developer Experience Requirements

- no cloud account required for local dev
- app boots locally with one command
- generated code is human-readable
- route mode is explicit
- direct writes outside actions are blocked
- destructive schema changes require explicit confirmation
- undo exists for major destructive operations where feasible

## Example Programming Model

Schema:

```ts
export const Post = entity("Post", {
  title: field.string(),
  slug: field.string().unique(),
  body: field.richtext(),
  authorId: field.id("User"),
  publishedAt: field.datetime().optional(),
})
```

Action:

```ts
export const publishPost = action("publishPost", {
  input: { postId: id("Post") },
  run: ({ db, input, auth }) => {
    const post = db.get("Post", input.postId)
    auth.require("update", post)
    return db.update("Post", input.postId, {
      publishedAt: new Date(),
    })
  },
})
```

Route:

```ts
export default defineRoute({
  path: "/blog/:slug",
  mode: "static",
  query: q.post.bySlug(),
  view: BlogPostPage,
})
```

## V1 Roadmap

1. Rust workspace and CLI shell
2. Schema DSL and validation
3. Codegen pipeline for TS bindings
4. Route manifest and static rendering
5. Postgres and SQLite adapters
6. Query and action execution runtime
7. Sync push/pull protocol
8. Web TS/React SDK
9. Mobile TS/React Native SDK
10. Auth runtime
11. Local Studio
12. Single-binary deploy path

## V1 Success Criteria

A strong engineer or agent should be able to:

- create an app in minutes
- create a content site in minutes
- add auth quickly
- define a live synced route quickly
- run the app locally with one command
- deploy the server as one binary
- debug policy and sync failures without reading framework internals
