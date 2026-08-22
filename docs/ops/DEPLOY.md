# Production deploy

pylon runs as a single binary behind a TLS-terminating reverse proxy.
This doc covers the supported self-hosted deploy shapes plus the
experimental Workers path.

## Required environment

```sh
# Core — pick ONE of DATABASE_URL or PYLON_DB_PATH:
PYLON_PORT=4321
DATABASE_URL=postgres://user:pass@host:5432/dbname   # multi-replica / managed
# PYLON_DB_PATH=/var/lib/pylon/pylon.db              # single-VPS / SQLite
PYLON_FILES_DIR=/var/lib/pylon/uploads
PYLON_MANIFEST=/etc/pylon/pylon.manifest.json

# Auth (MUST be set in non-dev)
PYLON_ADMIN_TOKEN=<64+ random bytes, hex>
PYLON_SESSION_DB=/var/lib/pylon/sessions.db  # SQLite only; Postgres shares auth state

# At-rest encryption key — used for SSO/SAML client secrets and any
# other sensitive values pylon persists. 32 bytes hex (or base64). When
# unset, pylon falls back to plaintext storage with a startup warning;
# fine in dev, refused effectively in production. Generate with
# `openssl rand -hex 32`. PYLON_SSO_ENCRYPTION_KEY is honoured as a
# legacy alias for backward compatibility.
PYLON_SECRET=<openssl rand -hex 32>

# Optional: declarative admin allowlist. Comma-separated list of
# verified email addresses. Matched users get auth.is_admin lifted on
# every request (case-insensitive, requires emailVerified=true). When
# the manifest also declares `auth: { user: { adminField: "..." } }`,
# the User row's flag is persisted on first match — removing the
# email from the env later does not demote (revoke explicitly via
# Studio / dashboard / SQL). PYLON_ADMIN_TOKEN remains for operator
# automation; this is for designating human admins.
PYLON_ADMIN_EMAILS=ops@your-domain.com,sre@your-domain.com

# Client-facing
PYLON_CORS_ORIGIN=https://your-app.com
PYLON_CSRF_ORIGINS=https://your-app.com

# Mode switch
PYLON_DEV_MODE=false
```

Optional:

```sh
PYLON_JOBS_DB=/var/lib/pylon/jobs.db          # SQLite job sidecar only
PYLON_WORKFLOWS_DB=/var/lib/pylon/workflows.db # SQLite workflow sidecar only
PYLON_OAUTH_GOOGLE_CLIENT_ID=...         # 25 builtin providers + any OIDC IdP
PYLON_OAUTH_GITHUB_CLIENT_ID=...         # see /auth/oauth for the full list
# Apple, Microsoft, Discord, Slack, Spotify, Twitch, Twitter, LinkedIn,
# Facebook, GitLab, Reddit, Notion, Linear, Vercel, Zoom, Salesforce,
# Atlassian, Figma, Dropbox, TikTok, PayPal, Kick, Roblox.
# OIDC: PYLON_OAUTH_<NAME>_OIDC_ISSUER=https://issuer (Auth0/Okta/etc).
PYLON_EMAIL_PROVIDER=stack0              # or sendgrid | resend | webhook
PYLON_EMAIL_API_KEY=sk_live_...          # provider API key
PYLON_EMAIL_FROM=noreply@yourdomain.com  # verified sender
PYLON_EMAIL_HTTP_URL=...                 # only when PYLON_EMAIL_PROVIDER=webhook

# File storage (defaults to local disk)
PYLON_FILES_PROVIDER=stack0              # local (default) | stack0 | s3
PYLON_STACK0_API_KEY=sk_live_...         # required when provider=stack0
PYLON_STACK0_FOLDER=uploads              # optional folder/prefix
PYLON_FILES_DIR=/var/lib/pylon/uploads   # local provider only
PYLON_FILES_URL_PREFIX=/api/files        # local provider only
# S3-compatible (AWS S3, R2, Tigris, MinIO). Boot-validated; never silently
# falls back to local disk. Credentials from env only (no IAM-role discovery).
PYLON_S3_BUCKET=my-bucket                # required when provider=s3
PYLON_S3_ACCESS_KEY=AKIA...              # required when provider=s3
PYLON_S3_SECRET_KEY=...                  # required when provider=s3
PYLON_S3_REGION=us-east-1                # optional, default us-east-1
PYLON_S3_ENDPOINT=https://...            # optional; R2/Tigris/MinIO (path-style)
PYLON_S3_PUBLIC_URL=https://cdn...       # optional; public bucket/CDN base
PYLON_S3_SESSION_TOKEN=...               # optional; STS/temporary credentials
```

Security hard requirements that fail to start:
- `PYLON_CORS_ORIGIN=*` in non-dev mode is refused
- `PYLON_DEV_MODE=true` with `PYLON_ADMIN_TOKEN` unset is refused
- `/api/__test__/reset` is disabled unless dev + in-memory + loopback

## Shape 1: single VPS (SSH + systemd)

Simplest and cheapest. One binary, one systemd unit, one reverse proxy.

```ini
# /etc/systemd/system/pylon.service
[Unit]
Description=pylon
After=network-online.target

[Service]
EnvironmentFile=/etc/pylon/env
ExecStart=/usr/local/bin/pylon serve
Restart=on-failure
RestartSec=5s
User=pylon
Group=pylon
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/pylon

[Install]
WantedBy=multi-user.target
```

Reverse proxy (caddy or nginx) forwards `:443` → `:4321` plus WebSocket
upgrades for `/ws` → `:4322`, `/events` (SSE) → `:4323`, and shard WS
→ `:4324`. A sample nginx config ships in `deploy/terraform/nginx.conf`.

```sh
systemctl enable --now pylon
```

Backups: cron `pylon backup /var/backups/pylon/$(date +%F)` nightly.
Test restore quarterly per the test at `crates/runtime/tests/backup_restore.rs`.

## Shape 2: AWS ECS + Aurora

`deploy/terraform/modules/pylon/` provisions:
- ECS Fargate service (0.25 vCPU, 512 MB) ~$9/mo
- Aurora Serverless v2 (0.5–2 ACU) ~$15/mo minimum
- ALB with TLS + WebSocket routing
- CloudFront CDN + Route53 DNS

Minimum bill: ~$25/mo for a production deployment.

`DATABASE_URL=postgres://...` is all the binary needs. `postgres-live`
is on by default for the runtime's storage dep, so no special build
flags. Apply schema before first start with `pylon migrate` (or let the
server auto-apply on boot, same as the SQLite path).

### Backend selection

The runtime picks its backend from the URL prefix:
- `postgres://` or `postgresql://` → live Postgres cluster
- anything else → SQLite filesystem path

`DATABASE_URL` takes precedence over `PYLON_DB_PATH` when both are set.

### Postgres runtime state

What works on Postgres today:
- Entity CRUD (`/api/entities/*`): insert / get / update / delete /
  list / list_after / lookup / link / unlink
- Filtered queries, graph queries, aggregations
- `/api/transact`: real PG transactions with auto-rollback on Drop
- Shared auth state, including sessions, OAuth state, magic codes, and API keys
- Shared jobs and workflows with leased claims and replica failover
- Multi-replica horizontal scaling for the data plane
- CRDT mode + FTS5 search: supported on both backends. CRDT uses
  per-row Loro snapshots stored in `_crdt_<entity>` (a separate table
  on Postgres, alongside the row table on SQLite). FTS5 maps to a
  `_fts_<entity>` table with `tsvector` columns + GIN index on
  Postgres, FTS5 virtual table on SQLite. Both maintained automatically
  on every insert/update/delete.

Jobs and workflow steps use at-least-once execution. Make their handlers
idempotent. Realtime change, presence, and CRDT fanout still needs
`PYLON_CLUSTER_BUS` on a self-hosted multi-replica deployment. See the
[horizontal scaling guide](../../apps/docs/operations/scaling.mdx).

## Shape 3: Cloudflare Workers (edge, experimental)

`crates/workers/` builds a WASM bundle (`worker-build --release`) that
runs on Workers with a D1 binding. See `crates/workers/README.md` for
current limitations. Scale-to-zero: idle apps cost $0. Cost rises with
actual request volume. See `docs/ops/WORKERS_COSTS.md`.

Realtime shards (tick-based sims) are not yet supported on Workers because
they need persistent state that Workers-only can't hold efficiently.
Use shape 1 or 2 for game shards.

## Shape 4: local dev

```sh
pylon dev
```

Starts on port 4321 with `PYLON_DEV_MODE=true` defaults. Studio at
`/studio`, hot-reload, permissive CORS. Not for production.

## Health checks

- `GET /health` returns 200 with `{"status":"ok","uptime_seconds":N}`
- `GET /metrics` returns Prometheus text when `Accept: text/plain`
- `GET /readyz` checks DB connectivity

Hook these into your load balancer and drain unhealthy instances.

## Graceful shutdown

Send `SIGTERM`. The server:
1. Stops accepting new connections
2. Lets in-flight HTTP requests finish (30s cap)
3. Closes WS + SSE with a normal close frame
4. Flushes the WAL
5. Exits with 0

For a rolling deploy, start the new instance, let the load balancer
promote it, send SIGTERM to the old one.

## Scale-out

Run multiple Pylon processes behind a load balancer. Use Postgres for
shared data, auth state, jobs, workflows, and sync state. Configure
`PYLON_CLUSTER_BUS` for realtime fanout. SQLite remains a single-machine
backend.

See the [horizontal scaling guide](../../apps/docs/operations/scaling.mdx).
