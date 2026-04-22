# Admin token rotation

`STATECRAFT_ADMIN_TOKEN` authenticates every privileged route:
- `/api/auth/session` in non-dev
- `/api/auth/upgrade` in non-dev
- `/api/admin/users/:id/export` (GDPR export)
- `/api/admin/users/:id/purge` (GDPR delete)
- `/api/sync/push`
- Jobs / workflows / scheduler control planes
- `/studio` in non-dev

Treat it like an SSH key: minimum 32 bytes of randomness, never in git,
rotate on any suspicion of compromise.

## Without downtime (two-token rotation)

The server only reads `STATECRAFT_ADMIN_TOKEN` at startup. Rotation requires
a restart. To do it without dropping traffic:

1. **Prepare**. Generate the new token:

   ```sh
   openssl rand -hex 32 > /etc/statecraft/admin_token.new
   ```

2. **Deploy side-by-side**. Start a new instance with the new token, let
   the load balancer health-check promote it, then drain + SIGTERM the
   old one. The 30-second in-flight window in `DEPLOY.md` covers admin
   calls that were mid-request.

3. **Update clients**. Any automation (CI, runbooks, cron, admin UIs)
   that hardcodes the old token must update. Grep for the old token
   prefix in Vault, 1Password, GitHub Actions secrets, Cloudflare
   environment, etc. before deleting the value.

4. **Verify + scrub**. Hit one admin endpoint with the new token; if it
   works, delete the old one from your secret store.

## Emergency (suspected compromise)

1. Generate a new token — skip no-downtime, it's not worth the risk:

   ```sh
   openssl rand -hex 32 > /etc/statecraft/admin_token.new
   ```

2. Revoke every active session and force re-login:

   ```sh
   curl -X POST -H "Authorization: Bearer $OLD_TOKEN" \
     https://your-host/api/admin/sessions/purge
   ```

   If you can't reach the admin API with the old token, stop the
   service and clear `STATECRAFT_SESSION_DB`:

   ```sh
   systemctl stop statecraft
   rm /var/lib/statecraft/sessions.db*
   systemctl start statecraft
   ```

3. Rotate OAuth secrets too — same blast radius if the admin account was
   used to configure them.

4. Audit `audit_log` for the period the old token was valid. The
   `audit_log` plugin records who did what and when.

5. File an incident report per `SECURITY.md`.

## What NOT to do

- Don't use the admin token as a session token — admin is not "a user",
  it's a break-glass credential.
- Don't commit the token to git, even in a test fixture. The
  pre-commit hook rejects 32+ hex strings in tracked files.
- Don't pass it as a URL query parameter. `Authorization: Bearer` only.
  URL params leak into proxy logs and browser history.
- Don't reuse the token across environments. Staging ≠ prod.
