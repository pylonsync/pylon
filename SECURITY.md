# Security policy

## Reporting a vulnerability

If you discover a security vulnerability, please **do not open a public issue**.
Instead, email **security@pylonsync.com** with:

- A description of the vulnerability
- Steps to reproduce
- Your assessment of impact and severity
- Any proposed fix

You'll receive an acknowledgement within 72 hours. We aim to ship a fix or
mitigation within 14 days for high/critical issues, 30 days for lower-severity.

## Supported versions

We support the most recent minor release. Older versions may receive security
patches at the maintainers' discretion.

## Dependency and image vulnerabilities

Advisories in Pylon's dependencies (crates, npm packages, Bun, the Debian base
image) are patched on these deadlines, counted from the advisory's publication:

| Severity | Fixed version available | Released | Running on Stack0 Cloud |
| --- | --- | --- | --- |
| Critical (CVSS 9.0+) | yes | 24 hours | 24 hours |
| High (CVSS 7.0 to 8.9) | yes | 72 hours | 72 hours |
| Medium and low | yes | next release | next fleet move |
| Any | no | ignored with a reason and a review date | |

What finds them:

- **CI** (`.github/workflows/ci.yml`, job `dependency advisories`): cargo audit,
  cargo deny and osv-scanner on every Rust or TypeScript change. Job
  `image scan`: Trivy on the runtime and builder images when a Dockerfile
  changes. Both fail on a high or critical vulnerability with a fixed version.
- **Dependabot** opens a security update when an alert opens, and weekly
  grouped minor and patch updates for cargo, bun, Docker base images and
  GitHub Actions.
- **The nightly scan** checks the published images and every Pylon version
  running on Stack0 Cloud against fresh advisory data.

An advisory with no fix, or one that does not apply, is ignored in the file
for its tool, with a reason and a review date:

- `.cargo/audit.toml` and `deny.toml` (the reason ends in "Review by
  YYYY-MM-DD"; `tools/check-security-ignores.sh` fails CI after that date),
- `osv-scanner.toml` (`ignoreUntil`),
- `.trivyignore.yaml` (`expired_at`).

The release and fleet steps are in `docs/ops/SECURITY_PATCHING.md`.

## Known hardening gaps (pre-1.0)

These are tracked and intended to be fixed before any 1.0 release. They are
documented here so deployers can make informed decisions:

- **OAuth token exchange uses an HTTP client with a 10s timeout.** DoS via
  slow OAuth providers is bounded but not zero. Deploy behind a reverse
  proxy that enforces request timeouts.
- **Sessions are in-memory by default.** Restart = logout. Set
  `PYLON_SESSION_DB=<path>` to persist sessions to SQLite.
- **Rate limiting is per-process.** Multi-instance deployments need an
  external rate limiter (nginx, Cloudflare, etc.).
- **Magic codes are 6 digits with 5-attempt cap and 60-second cooldown.**
  Code space is 10^6; 5 attempts means ~5e-6 chance of guessing per code.
  This is deliberate: codes are short enough to type from an email.
- **CORS defaults to `*` in dev mode.** Set `PYLON_CORS_ORIGIN` to a specific
  origin in production.
- **Workers deployment is experimental.** Do not run sensitive workloads on
  the Workers target until it is marked stable.
- **The TypeScript function runtime runs untrusted code via Bun.** Treat
  `functions/*.ts` as server-side code with full database access. Do not
  load code you don't control.

## Secure defaults in production

Set at minimum:

- `PYLON_ADMIN_TOKEN` — admin API token (long random string)
- `PYLON_CORS_ORIGIN` — specific origin, not `*`
- `PYLON_DEV_MODE=false`
- `PYLON_SESSION_DB=/var/lib/pylon/sessions.db`
- `PYLON_DB_PATH=/var/lib/pylon/pylon.db`
- `PYLON_RATE_LIMIT_MAX=30` (anon per-IP — default 100 is lenient for auth surfaces)
- `PYLON_RATE_LIMIT_MAX_AUTHED=1000` (authed per-user — bump higher for polling dashboards)

Recommended:

- Run behind a reverse proxy that terminates TLS
- Mount the data directory as a persistent volume
- Enable `PYLON_EMAIL_PROVIDER=sendgrid`, `resend`, or `stack0` for magic-code delivery
- Back up the database regularly (`pylon backup <dir>`)
