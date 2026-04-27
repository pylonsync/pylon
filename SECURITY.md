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
- `PYLON_RATE_LIMIT_MAX=30` (default 100 is lenient)

Recommended:

- Run behind a reverse proxy that terminates TLS
- Mount the data directory as a persistent volume
- Enable `PYLON_EMAIL_PROVIDER=sendgrid`, `resend`, or `stack0` for magic-code delivery
- Back up the database regularly (`pylon backup <dir>`)
