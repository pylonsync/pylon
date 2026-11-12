Security review — 2026-10-27

Reviewed commit: `7d30953c12dbcd222b2fcd38b38f54bb8df9850d`.

This review found three high-priority issues. Two were reproduced through HTTP against a local server built from this commit. The third follows from the B2B template handlers and the default function policy behavior. Application fixes remain open. No review can prove that an app cannot be hacked.

The existing untracked `SECURITY_AUDIT_2026-10-26.md` was preserved. It covers this same commit. Its operator findings were checked again. This review adds HTTP evidence, the Studio ticket path, the B2B template finding, and current dependency results.

**1. High: a metrics token can gain Studio administration access**

Evidence: `crates/runtime/src/server.rs:5410`, `:5465`, and `:10375`.

Both `POST /admin/studio-ticket` and `/admin/operators` use `verify_admin_or_metrics_auth`. That helper accepts `PYLON_METRICS_TOKEN`. A metrics-token holder can request a Studio ticket or create an operator with an attacker-chosen password. Operator credentials grant deployment administration access. The token therefore has more authority than its stated read-only role.

The HTTP probe used `PYLON_DEV_MODE=0`, separate admin and metrics tokens, and an explicit CORS origin. Results:

- Anonymous operator creation: 401.
- Metrics-token operator creation: 201.
- Metrics-token Studio ticket request: 200.
- Ticket redemption returned a Studio session cookie. That cookie authorized operator changes.

This attack requires access to a configured metrics token and the affected endpoint. It does not require an existing administrator session. The entity browse routes also deliberately accept the metrics token and bypass row read policies. Treat existing metrics credentials as sensitive data-access credentials even after the write escalation is fixed.

Fix: use an admin-only authorization helper for ticket issuance and operator management. Keep metrics access limited to explicitly approved read routes. Add denial tests for metrics-token ticket issuance, operator creation, password changes, and deletion. Keep positive tests for authorized administrators.

**2. High, conditional: operator changes accept forged browser requests**

Evidence: `crates/runtime/src/server.rs:5465`, `:5492`, `:5532`, `:6241`, and `:10412`; `crates/auth/src/cookie.rs:78`.

Operator routes accept a Studio or application admin cookie. They return before the main dispatcher checks the request Origin. Their JSON reader also accepts `Content-Type: text/plain`.

The HTTP probe redeemed a Studio ticket, then sent only its session cookie with `Origin: https://evil.example.test`. The configured allowed origin was `https://app.example.test`. Both requests used a text/plain JSON body:

- Operator creation returned 201.
- An existing operator password change returned 200.

This proves that the server accepts the forged request shape. A browser attack additionally requires the browser to send the victim cookie. The default SameSite=Lax setting limits cross-site POST requests. It does not stop an attacker on a sibling origin within the same site. SameSite=None can allow broader attacks. CORS does not prevent a simple request from changing server state. See [MDN's CSRF guidance](https://developer.mozilla.org/en-US/docs/Web/Security/Attacks/CSRF).

The probe used an HTTP client with an explicit cookie header. It did not run a browser exploit. The victim must have an authorized session for an independent CSRF attack. The metrics flaw is not a required condition.

Fix: apply the shared CSRF check before every mutation that accepts cookie authentication. CSRF means cross-site request forgery. Preserve explicit bearer-token access for server clients. Reject unsupported content types as a separate control. Add tests that verify both the rejection response and the unchanged account store.

**3. High: B2B template functions omit organization authorization under default settings**

Evidence:

- `packages/create-pylon/templates/backend/b2b/apps/api/functions/setMemberRole.ts:28`.
- `packages/create-pylon/templates/backend/b2b/apps/api/functions/inviteMember.ts:31`.
- `packages/create-pylon/templates/backend/b2b/apps/api/functions/orgMembers.ts:11`.
- `crates/functions/src/runner.rs:1718` and `:3213`.
- `packages/functions/src/define.ts:287`.

The template handlers rely on entity policies to restrict organization access. They do not check the caller's membership or role. The runtime only enables function database policy checks when `PYLON_STRICT_FN_POLICIES` equals `1`. The default function authentication mode requires a signed-in user. It does not require an organization administrator.

Under the default setting, a signed-in caller who knows the relevant IDs can call `setMemberRole` to change a membership to owner. `inviteMember` can insert a membership in a foreign organization. `orgMembers` can read its membership list. These handlers use ordinary `ctx.db` operations. They do not need the explicit unsafe API to bypass the policies under this default.

The existing unit test `runner::tests::gate_skipped_when_strict_off` passed. It verifies that the function database path skips the policy gate with strict mode off. The handler source confirms the missing caller checks. A generated B2B app was not tested over HTTP. This finding applies to apps that use these handlers without added checks and without strict function policies. It is not evidence that every app in the repository has this flaw.

Fix: check the caller's membership and required role inside each public function before access. Cover invite, role change, removal, project changes, and organization reads. Add tests for an outsider, an ordinary member, an administrator, and a caller from another organization. Do not treat enabling strict mode as a complete template fix without tests. The initial owner membership in `createOrg` needs a controlled bootstrap path because the membership policy requires an existing administrator.

**4. Low: operator request bodies have no size cap**

Evidence: `crates/runtime/src/server.rs:5492` and `:7522`.

The operator handler calls `read_to_end` without a byte limit. It runs before the general HTTP body limit. An accepted caller can cause a large allocation. This is an authenticated denial-of-service risk. No resource-exhaustion test was run.

Fix: use a bounded body reader. Return 413 for an oversized body. Test declared length and streamed bodies. Apply a request timeout at the server or proxy.

**5. Medium process gap: source scans omit tracked lockfiles**

Evidence: `.github/workflows/ci.yml:488` and `.github/workflows/security-daily.yml:87`.

Both source scan commands list only the root Cargo lockfile, root Bun lockfile, and Studio Bun lockfile. Separate Docker, benchmark, shard, and vendor lockfiles fall outside those lists. Image scans provide separate coverage. They do not make the source scan complete.

Fix: enumerate tracked supported lockfiles. Record deliberate exclusions and their reasons. Separate production results from benchmark and standalone vendor results.

**Dependency results**

`cargo audit --json` returned exit 0. The RustSec database commit was `99670cfc0bc574433034b75876a4b791ccaa2efb`, updated on 2026-10-27. It reported no unsuppressed vulnerabilities, seven unmaintained packages, and two unsoundness warnings. The repository's advisory ignores were active. This result does not establish that dependencies are safe.

The recursive OSV scan returned exit 1 with advisory matches. It inspected supported files with transitive manifest resolution disabled. New matches compared with the prior report include:

- `satori 0.26.0` in the root and Docker framework lockfiles. The advisory describes improper escaping in SVG output. It lists 0.33.5 as fixed. Pylon's inspected OG renderer converts the SVG to PNG before returning it. Browser script execution through this path was not established. Update the dependency and Docker lockfile, then test image output. See [the Satori advisory](https://github.com/advisories/GHSA-wx4j-mvgx-mqwp).
- `block-buffer 0.10.4` and `0.12.0` in the root lockfile. The advisory describes memory corruption after a caught panic leaves a buffer position invalid. It lists 0.12.1 as fixed. The scan alone does not prove that Pylon reaches that failure path. Trace each dependency chain before choosing compatible updates. See [the block-buffer advisory](https://github.com/advisories/GHSA-qwgh-2vcv-g2f7).

Other matches remain in the benchmark TLS dependencies and the vendor-local OpenSSL, quick-xml, and rand dependencies. The root lockfile has newer TLS and OpenSSL versions. Do not report a vendor-local or benchmark version as a production binary finding without tracing the build.

The root dependency warnings include `im` and `sized-chunks` memory-safety advisories. Remote exploitability was not established. The RSA advisory remains ignored by repository policy. The audit command therefore does not certify RSA as patched.

**Verification and scope**

Commands completed:

```sh
cargo audit --json
osv-scanner scan source -r --no-resolve --format json --output-file /tmp/pylon-review-2026-10-27-osv.json .
cargo test -p pylon-runtime --test security_review_probe_20261027 -- --nocapture
cargo test -p pylon-functions --lib gate_skipped_when_strict_off
cargo test -p pylon-runtime --test bearer_cookie_fallback --test update_policy_scope --test file_serving --test fn_sse_auth_gate --test sync_relay_token_unconfigured
```

The temporary HTTP probe passed its assertions that the vulnerable behavior occurs. That is exploit evidence, not a passing security requirement. Its first run timed out before receiving an HTTP response. The successful run used memory-only job and session stores, disabled function loading, and allowed a longer client timeout.

The function policy unit test passed. All 29 tests across the five selected regression suites passed. They cover bearer/cookie handling, row and tenant policy checks, uploaded-file response controls, function SSE access, and an unconfigured sync relay. They do not cover the operator flaws above.

The source review also sampled JWT validation, remote image fetch guards, private-field projection, and CRDT privacy handling. CRDT is the replicated data format used for shared documents. These samples do not establish complete coverage of those features.

The HTTP probe source is `/tmp/pylon-review-probe-20261027.rs`. Its log is `/tmp/pylon-review-probe-20261027.log`. To repeat it, copy the source to `crates/runtime/tests/security_review_probe_20261027.rs` and run the command above. The temporary test file was removed from the repository after the run. Dependency JSON files and test logs use the `/tmp/pylon-review-` prefix. These temporary files are local evidence and are not permanent repository artifacts.

No application source or dependency files were changed. No production service was tested. The review did not inspect deployed secrets, cloud permissions, running images, native system libraries, every function or template, or full Git history for secrets. The full test suite was not run. These remain separate verification tasks.

Fix the three high-priority issues first. Add HTTP denial tests before release. Then bound operator bodies, update affected dependencies, and extend source-scan coverage. Check deployed apps for affected templates and configuration after the fixes are ready.
