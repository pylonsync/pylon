# Incident response

## Reporting

**Security vulnerabilities**: email `security@statecraft.dev`. Do NOT file a
public issue. Acknowledgement within 48 hours. Target remediation 7 days
for high severity, 30 days for medium.

**Operational incidents** (your own deploy): follow your internal
runbook. This doc is for anchoring the generic moves.

## First five minutes

1. **Contain**. If the blast radius is unknown, flip the relevant feature
   flag or route to maintenance mode. A 503 is better than a breach.

2. **Preserve**. Snapshot the database before any destructive recovery:

   ```sh
   statecraft backup /var/snapshots/incident-$(date +%Y%m%d-%H%M%S)
   ```

3. **Capture signal**. Pull logs, metrics, and recent audit trail:

   ```sh
   journalctl -u statecraft --since "30 min ago" > /tmp/incident.log
   curl -s http://localhost:4321/metrics > /tmp/incident-metrics.txt
   ```

4. **Declare**. Spin up an incident channel. One person owns coordination,
   one person drives fixes. Everyone else stays off.

## Common incidents

### Admin token leaked

Follow `TOKEN_ROTATION.md` → emergency path. Revoke every session, rotate
the token, audit the window of exposure. If OAuth credentials were stored
as admin-level env vars, rotate those too.

### Policy bypass / data exposure

1. Reproduce with a throwaway session token against staging.
2. Check `audit_log` for rows accessed during the window — this is the
   GDPR notification trigger in the EU.
3. Patch, ship, verify the regression test in
   `crates/policy/src/lib.rs::tests` covers it.
4. If user data was exposed: initiate breach notification workflow.

### Runaway write loop

Signal: write rate pegged above the 70k/sec ceiling, WAL growing
unbounded, disk filling.

1. Identify the source: check recent deploys + `audit_log` for the
   offending `user_id` or IP.
2. Rate-limit or block at the proxy — don't try to fix inside the server
   while it's under load.
3. Once stable, investigate the trigger. Common cause: a client in an
   exponential-backoff retry loop without jitter.

### WAL file growth past disk budget

SQLite in WAL mode delays checkpoints. If checkpoint isn't completing:

```sh
# Offline checkpoint — needs exclusive DB access.
systemctl stop statecraft
sqlite3 /var/lib/statecraft/statecraft.db "PRAGMA wal_checkpoint(TRUNCATE)"
systemctl start statecraft
```

If the WAL is *always* growing, a long-running read transaction is
holding it open. Find it via `sqlite3 … ".pragma stats"` and kill the
client holding the lock.

### WS fanout storm

Signal: ws-broadcast-N threads at 100% CPU, clients reporting missed
events, queue-full warnings in the log.

1. Check client count — per-IP cap is 64; if one IP has 64, a client is
   looping.
2. Check broadcast rate. The event that's being broadcast might not be
   expected — e.g. a retry loop creating 1000 inserts/sec.
3. Bounce the ws server (`kill -HUP <pid>`) only as a last resort —
   every client has to reconnect.

### Cloudflare Workers billing spike

Signal: CF email saying you've used 80% of your monthly budget in a
week. See `docs/ops/WORKERS_COSTS.md` for patterns.

## Post-mortem template

1. **Summary** — one paragraph, what happened, duration, blast radius.
2. **Timeline** — UTC timestamps from first signal to all-clear.
3. **Root cause** — what went wrong, not who.
4. **What worked** — detection, containment, comms.
5. **What didn't** — slow alerts, unclear runbooks, missing graphs.
6. **Action items** — with owners and target dates. Track in the sprint.

Publish internally within 5 business days. External disclosure for
user-impacting incidents per your privacy policy.

## Contacts

- Security: `security@statecraft.dev`
- Public issues / feature requests: github.com/ericc59/agentdb/issues
- Your oncall: (fill in your rotation)
