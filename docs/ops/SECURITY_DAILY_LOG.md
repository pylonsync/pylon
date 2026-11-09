# Daily security log

## 2026-10-28

Reviewed commit: `7d30953c12dbcd222b2fcd38b38f54bb8df9850d`.

The commit has not changed since the 2026-10-27 review. At the start, 21 tracked files had local changes. Seven untracked files held prior reports, regression tests, and scanner code. The prior runtime, B2B template, dependency, and workflow fixes remain local. They were preserved. They are not reported as new open defects. No commit, push, release, deployment, or external message was made.

Read `SECURITY.md`, both 2026-10-27 reports, the 2026-10-26 report context, and the security workflows. The existing local daily task is set for 09:00. No duplicate task was created.

### Review scope

Today's main area was files, server-side request forgery (SSRF), and path traversal. Traced upload initialization, owner binding, local PUT, confirmation, download, and deletion in `crates/runtime/src/server.rs`. Reviewed local storage and private/public S3 behavior, signed download URLs, frontend path checks, and remote image fetching. Checked redirect validation, the guarded DNS resolver, byte limits, and download response types. Reviewed the marketplace upload template and AI Studio provider calls. No new exploitable issue was confirmed in the inspected runtime file paths.

The review also checked the existing operator and B2B fixes. Their selected regression tests still pass. The prior metrics-token read access remains intentional behavior. Protect that token as a data-access credential.

### Confirmed finding and fix

**Medium: AI Studio permits changes to another user's generation through public polling.**

Before this change, `packages/create-pylon/templates/ai-studio/functions/pollGeneration.ts` declared `auth: "public"`. It used `_getGeneration` and `_updateGeneration`, which use the unsafe database API. It did not check the generation owner. Entity read policies did not protect this function path.

An attacker needs the ID of a pending or processing generation and access to the function endpoint. No session or provider API token is required. A call with `{ "id": "<victim-id>", "attempts": 201 }` reaches the timeout branch and marks that generation as failed. This can interrupt another user's work. The review did not establish a way to enumerate private generation IDs. It did not establish data theft or remote code execution.

The new test loads the real polling, read, and update handlers with a local test database. Before the fix, both anonymous and foreign-user denial tests failed because the calls succeeded. The auth-mode test also failed. The three owner/retry tests passed. This is handler-level reproduction, not a full generated app HTTP exploit.

The fix requires a user session and checks `gen.userId` before any update, provider request, or reschedule. `_getGeneration` now includes `userId` in its return type. The scheduler retains the original caller identity. This was traced through `ScheduleCallerInfo`, `JobAuth`, and job dispatch in `crates/runtime/src/datastore.rs`. No authorization control was weakened.

Added `packages/functions/src/ai-studio-template-security.test.ts` to the existing CI security test command. Its six tests cover anonymous and foreign-user denial, unchanged records after denial, owner completion, owner timeout, completed-job retries, and the HTTP auth mode.

### Advisory scans

All 12 tracked supported lockfiles were scanned with OSV Scanner 2.7.0 and current OSV data. `tools/scan-lockfiles.py` returned 1 for findings. `tools/osv-gate.py` returned 0 under the existing severity policy. The scan is not clean. Comparing package, version, advisory, and lockfile tuples with the prior fix scan found no new or removed matches.

| Scope | Result |
| --- | --- |
| Production/root Rust | 10 OSV matches in `Cargo.lock` |
| Root Bun and Studio Bun | No reported matches |
| Docker/build | No reported matches in the three Docker Bun lockfiles |
| Benchmark | 9 matches in `benchmarks/search/Cargo.lock` |
| Vendor-only | No reported matches in `vendor/samael/Cargo.lock` |
| Shard and Swift examples | No reported matches in the four lockfiles |

`cargo audit --json` returned 0 with existing ignores. RustSec database commit: `40f5be6fb12ed93fc591bf4ab23b7d793b3f2239`, updated 2026-10-28. It reported zero unsuppressed vulnerabilities, seven unmaintained packages, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock consistency and advisory-ignore date checks passed. No dependency or ignore was changed today.

Remaining warnings include:

- `block-buffer 0.10.4`: `GHSA-qwgh-2vcv-g2f7`, CVSS 6.3. Today's OSV output provides no fixed release for this matched range. The prior source review did not establish the reported failure in this version. Obtain upstream clarification or migrate the affected dependency chains. A compatible update to 0.12 cannot replace 0.10 callers.
- `im 15.1.0`: `RUSTSEC-2023-0126`, aliasing violation. `sized-chunks 0.6.5`: `RUSTSEC-2026-0255`, panic-safety memory errors. No fixed releases are reported. Remote exploitation was not established. No validated runtime mitigation was found. A dependency migration needs separate compatibility tests.
- Unmaintained packages: `atomic-polyfill`, `bincode`, `bitmaps`, `im`, `paste`, `rustls-pemfile`, and `sized-chunks`.
- The existing RSA exception, `RUSTSEC-2023-0071`, remains active. Passing gates do not mean it is patched.

### Verification

Final checks passed:

| Command | Result |
| --- | --- |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 tests |
| `cargo test -p pylon-kernel net_guard` | 6 tests |
| `cargo test -p pylon-storage --lib files::` | 31 tests |
| `cargo test -p pylon-storage --lib s3::` | 14 tests |
| `cargo test -p pylon-runtime --lib image_optim::` | 24 tests |
| `cargo test -p pylon-runtime --lib file_urls::` | 4 tests |
| `cargo test -p pylon-runtime --lib frontend::tests::resolve_safe` | 2 tests |
| `cargo test -p pylon-runtime --test admin_mutation_security --test file_serving --test bearer_cookie_fallback --test scheduler_new_function` | 14 tests |
| `bun test packages/functions/src/ai-studio-template-security.test.ts packages/functions/src/b2b-template-security.test.ts packages/functions/src/member.test.ts packages/functions/src/ssr-og-image.test.ts packages/functions/src/runtime-files.test.ts` | 30 tests with Bun 1.4.4 |
| `bun run --cwd packages/functions check` and `build` | Both passed with Bun 1.4.4 |
| `tsc --noEmit -p /tmp/pylon-security-20261028-template-tsconfig.json` | AI Studio function types passed against the local SDK |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 tests |
| `git diff --check` | Passed |

Initial scans failed because the sandbox blocked advisory-cache writes or network access. They were rerun successfully with the required access. Initial storage, image, and runtime tests failed because local socket binding was blocked. Their final runs passed with socket access. The default Bun was 1.3.14. It could not load an existing B2B test's data URL. The final TypeScript suite used the pinned Bun 1.4.4 and passed. No test was disabled. Logs and scan JSON are under `/tmp/pylon-security-20261028-*`.

The final source diff was reviewed. No repository-wide formatter configuration was found. The changed TypeScript follows the surrounding style. The full workspace test suite and full application build were not run. Container images, live deployments, cloud permissions, and system libraries were not scanned. The Nix `vendor/samael/flake.lock` requires separate tooling. These gaps are not clean results.

### Existing apps and next review

Apply both changed AI Studio handler files to existing generated apps. Regenerate the app manifest and deploy through the normal release process. Jobs with no saved caller identity will now be rejected. Check such pending jobs and restart them through the authenticated generation flow. No existing app was changed by this repository edit.

The prior runtime and B2B fixes still need release and deployment. Existing B2B apps need the handler changes. Application lockfiles also need their own scans and patched Satori/fflate versions, as described in `SECURITY_FIXES_2026-10-27.md`.

Next main area: server functions and generated app templates. Review scheduler entry points, internal helper access, caller identity, and owner/tenant checks beyond AI Studio and B2B.

## 2026-10-29

Reviewed commit: `f17c702f45d2145b6ecb45bfd1a27e2f3128ae19`.

The previous successful review used `7d30953c12dbcd222b2fcd38b38f54bb8df9850d`. One commit changed seven files since that review. The working tree started with 23 modified tracked files and nine untracked files. These include the prior runtime, template, dependency, and workflow fixes. All were preserved. Those fixed defects are not reported as new open issues. This run only appended this log entry. No new code or dependency fix was needed for a confirmed finding. No commit, push, release, deployment, or external message was made.

The existing daily task remains set for 09:00 in the local time zone. Its next run is October 30. Its prompt includes the review and fix instructions. No duplicate task was created.

### Review scope and result

No new exploitable issue was confirmed in the inspected paths. Read `SECURITY.md`, the prior reports, and the CI, daily scan, and AI review workflows. Checked the existing local runtime fix and its HTTP regression.

The main area was server functions and generated app templates:

- Traced HTTP function and webhook dispatch in `crates/router/src/routes/functions.rs`. Checked internal-function rejection and public, guest, user, and admin authentication modes.
- Traced nested calls and caller identity in `crates/functions/src/runner.rs` and `crates/runtime/src/datastore.rs`. Nested server calls retain caller identity. Application server code remains trusted code with database access. The HTTP authentication gate does not replace checks in that code.
- Checked the scheduler's internal-target restriction, saved `JobAuth`, transaction buffering, and job dispatch. Jobs retain the captured user, tenant, guest, and admin fields. Persisted jobs do not retain role lists. Dispatch supplies an empty role list. This limitation remains; functions that need current roles must read them at execution time.
- Reviewed backend consumer profile, post, follow, and demo-seed handlers. Checked owner-derived identifiers and the seed lock. Reviewed mobile note creation and account-data deletion. The deletion helper is internal and checks the caller against the supplied user ID.
- Reviewed backend chat handlers, project task/time handlers, marketplace offer responses, and selected iMessage relay and internal-helper paths. The project template explicitly uses one signed-in team as its trust boundary. The relay verifies its configured token before elevation. The inspected internal note helper checks the contact ID.
- Rechecked the prior B2B and AI Studio fixes with their actual-handler tests. These remain local fixes, not new findings.

Also reviewed the new sync commit. It retains old row data when read-policy fields, sync-scope fields, or dependency-source entities need it. Broken policy compilation retains the old data. Reviewed the row-path traversal and bounded change-log loading. The selected policy, sync, dependency-hint, and SQLite replay tests passed. This was a selected-path review, not a review of all 186 template function files.

### Current advisory checks

OSV Scanner scanned all 12 tracked supported lockfiles. The scan returned 1 for findings. `tools/osv-gate.py` returned 0 under the existing severity policy. Advisory tuples are unchanged from the October 28 scan. No ignore or dependency was changed. This scan is not clean.

| Scope | Result |
| --- | --- |
| Production/root Rust | 10 advisory groups in `Cargo.lock` |
| Root Bun and Studio Bun | No reported matches |
| Docker/build | No reported matches in the three Docker Bun lockfiles |
| Benchmark | 9 advisory groups in `benchmarks/search/Cargo.lock` |
| Vendor-only | No reported matches in `vendor/samael/Cargo.lock` |
| Shard and Swift lockfiles | No reported matches in four lockfiles |

`cargo audit --json` returned 0 with the existing ignores. It used RustSec commit `ee1a49a2769a24a596afc1c20cb33748d2769586`, updated October 29. It reported zero unsuppressed vulnerabilities, seven unmaintained-package warnings, and two unsoundness warnings. `cargo deny check` returned 0. Image-lock consistency and advisory-ignore date checks passed.

Remaining risks are unchanged:

- `block-buffer 0.10.4` still matches `GHSA-qwgh-2vcv-g2f7`. The [current advisory](https://osv.dev/vulnerability/GHSA-qwgh-2vcv-g2f7) lists 0.12.1 as fixed. That version is not a compatible replacement for 0.10 callers. The earlier source review did not establish the inline-position failure in 0.10.4. Keep the finding visible pending upstream clarification or dependency migration.
- `im 15.1.0` and `sized-chunks 0.6.5` retain memory-safety warnings. The [current sized-chunks advisory](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) reports no patched version. It requires a panic in element destruction. Remote exploitation in Pylon was not established. No validated runtime mitigation was found. Migration needs compatibility tests.
- The seven maintenance warnings and the existing `RUSTSEC-2023-0071` RSA exception remain. Passing gates do not patch these dependencies.

### Verification

| Command | Final result |
| --- | --- |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `cargo test -p pylon-policy -p pylon-sync --lib` | 107 policy and 45 sync tests passed |
| `cargo test -p pylon-functions --lib` | 89 passed |
| `cargo test -p pylon-router --lib routes::functions` | 5 passed |
| `cargo test -p pylon-runtime --test scheduler_new_function --test sync_dependency_hint --test sync_scope_filter_parity --test sqlite_replay_readers --test admin_mutation_security` | Harness reported 7 passed; one PostgreSQL test returned early because `PYLON_TEST_PG_URL` was absent. Six tests exercised their paths. |
| `bun test packages/functions/src/ai-studio-template-security.test.ts packages/functions/src/b2b-template-security.test.ts packages/functions/src/member.test.ts` | 24 passed with Bun 1.4.4 |
| `bun run --cwd packages/functions check` and `build` | Both passed with Bun 1.4.4 |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 passed |
| `git diff --check` | Passed |

Initial Cargo scans failed on advisory-cache permissions. The initial OSV scan failed on DNS access. The reruns with required access completed. The first runtime test run failed because the sandbox blocked local socket binding. The rerun passed. Default Bun 1.3.14 failed to load the B2B test's data URL. Bun 1.4.4 was installed in a temporary directory, and the final tests passed. No test was disabled. Existing Rust compiler warnings remain. Logs and scanner output are under `/tmp/pylon-security-20261029-*`.

The final working-tree status and diff were checked. No application source changed in this run, so no new before/after regression was required. The full workspace tests and full application build were not run. PostgreSQL integration, container images, live deployments, cloud permissions, system libraries, and the Nix `vendor/samael/flake.lock` were not tested or scanned. These gaps are not clean results.

### Existing apps and next review

The prior runtime and dependency fixes still need release and deployment. Existing B2B and AI Studio apps need the handler changes described in the earlier entries. Regenerate their manifests and scan their own lockfiles. Update independently installed Satori and fflate dependencies as described in `SECURITY_FIXES_2026-10-27.md`. Review old AI Studio jobs without saved caller identity. This run adds no new deployment step.

Next main area: secrets and deployment configuration. Check production defaults, generated environment files, secret handling in logs, and runtime/build image configuration.

## 2026-11-01

Reviewed commit: `f17c702f45d2145b6ecb45bfd1a27e2f3128ae19`.

The last successful logged review was October 29. HEAD has not changed. The run started with 24 modified tracked files and 10 untracked status entries. These include prior security fixes, reports, scanner code, and a deployment regression test. All prior changes were preserved. Existing fixed issues are not new open findings. The root lockfile already contains Wasmtime 49.0.5. The benchmark still used 49.0.4. No commit, push, publication, deployment, or external message was made.

Read SECURITY.md, both October 27 reports, the daily log, and the security workflow configuration. Confirmed the existing daily task runs at 09:00 local time. Its next run is November 2. No duplicate task was created.

### Scope and confirmed fix

The main area was secrets and deployment configuration. Reviewed runtime development-access rules, production origin checks, cookie security, rate-limit defaults, image settings, and generated Docker, Fly, Compose, and systemd files. Checked generated Docker exclusions and CLI secret masking. The CLI masks configured sensitive values in both text and JSON output. This was a selected source review, not a scan of all secrets or all deployment platforms.

**Unsafe deployment default:** the runtime Dockerfile set `PYLON_DEV_MODE=true`. Generated Dockerfiles inherited this setting. It disables Secure cookies by default and raises default request limits to 100,000 per minute. It also bypasses the production requirement for trusted origins. These effects apply even when the separate loopback checks deny development shortcuts.

Owner-level development access additionally requires a loopback peer and a loopback Host, or an explicit remote-trust setting. A remote attacker can meet the loopback conditions through a local proxy that rewrites Host to localhost. An absent Origin is accepted for non-browser clients. Direct remote requests do not automatically receive these privileges. Source tracing confirmed these conditions in `dev_access.rs`. This run did not reproduce a complete proxy attack or run a Docker container.

Both the base image and generated Dockerfile now set `PYLON_DEV_MODE=false`. The generated setting also protects apps that use older base images. Production startup requires trusted origins through the manifest or `PYLON_CORS_ORIGIN`. SECURITY.md records this change. The existing local deployment regression test failed before the fix with `Some("true")` instead of `Some("false")`. It passed after the fix. Its formatting was corrected without changing its assertions.

### Dependencies and current advisories

Updated the standalone benchmark lockfile from Wasmtime 49.0.4 to 49.0.5 with Cargo. Matching Wasmtime and Cranelift packages moved together. No dependency API change was required. The benchmark compiled with the updated lockfile. This removes its matches for RUSTSEC-2026-0422, RUSTSEC-2026-0423, and RUSTSEC-2026-0424. The last advisory describes unbounded host memory use through component resource borrowing. Exploitation requires execution of a guest that uses the affected path. Benchmark exploitability was not reproduced. See the [upstream advisory](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-6fr9-xmw9-5352).

OSV Scanner 2.7.0 scanned all 12 tracked supported lockfiles with current data. Both full scans returned 1 for findings. The final severity gate returned 0. The scan is not clean.

| Scope | Final OSV result |
| --- | --- |
| Production/root Cargo | 9 advisory groups |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build dependency locks | No reported matches in all three locks |
| Benchmark | 8 advisory groups after the Wasmtime update |
| Vendor-only SAML | No reported matches |
| Shard and Swift locks | No reported matches in four locks |

Cargo audit returned 0 with existing ignores. RustSec database commit: `2295466c4e9df26647fd67d8fa1c705ac2fc7f81`, last updated October 30. It reported zero unsuppressed vulnerabilities, seven unmaintained warnings, and two unsoundness warnings. Cargo deny passed advisories, bans, licenses, and sources. No ignore was added or extended.

Remaining risks and follow-up:

- Studio's ESLint and TypeScript ESLint tools resolve minimatch 10.2.5. GHSA-g3hv-75wg-m249 reports a fixed release at 11.0.1. The callers require version 10. The [advisory](https://github.com/advisories/GHSA-g3hv-75wg-m249) requires attacker-controlled glob patterns with `optimizationLevel >= 2`. No production HTTP path to these build tools was established. Keep build configuration and glob inputs trusted. Upgrade the caller packages or validate a major-version override with Studio lint and build checks. This run did not validate that migration, so the finding remains open.
- `im 15.1.0` and `sized-chunks 0.6.5` retain memory-safety warnings without fixed releases. Remote exploitation was not established. No validated runtime mitigation is available from this review. Migration and compatibility tests remain required.
- Maintenance warnings remain for atomic-polyfill, bincode, bitmaps, im, paste, rustls-pemfile, and sized-chunks. The existing RSA exception remains active. A passing gate does not patch these packages.
- The previous block-buffer 0.10.4 match is absent from today's current OSV results. No suppression or dependency change removed it in this run.

### Verification and gaps

- `cargo test -p pylon-cli deployment_images_disable_dev_access`: failed before the fix, as expected.
- `cargo test -p pylon-cli commands::deploy::tests`: 11 passed after the fix.
- `cargo test -p pylon-runtime --lib dev_access`: 6 passed.
- `cargo test -p pylon-runtime --test admin_mutation_security --test dev_network_access`: passed against local test servers.
- `cargo check --locked --manifest-path benchmarks/search/Cargo.toml`: passed.
- `python3 -m unittest discover -s tools -p 'test_*.py'`: 3 passed.
- `rustfmt --edition 2021 --check crates/cli/src/commands/deploy.rs`: passed after formatting the pre-existing test.
- `bash tools/check-image-locks.sh`, `bash tools/check-security-ignores.sh`, and `git diff --check`: passed.

Cargo and rustc were 1.98.1. The initial advisory checks and dependency downloads hit sandbox restrictions. Reruns with required access completed. Existing Rust compiler warnings remain. Logs and scan output are under `/tmp/pylon-security-20261101-*`.

The source diff and lockfile update output were inspected. The full workspace tests, TypeScript checks, Studio build, and Docker image build were not run. This run changed no TypeScript source or Bun lockfile. Live deployments, cloud permissions, container contents, system libraries, and Nix flake inputs were not scanned. These gaps are not clean results.

### Existing deployments and next review

Set `PYLON_DEV_MODE=false` on existing deployments now. Configure trusted origins and an admin token. Rebuild existing app Dockerfiles with the explicit setting. Release updated base images through the normal process. Explicit platform environment overrides still take precedence. Development containers that need development mode must opt in deliberately. No deployed app was changed here.

The prior runtime, B2B, and AI Studio fixes still need release or application updates. Existing generated apps need their handler changes and regenerated manifests. Scan their independent lockfiles. Check old AI Studio jobs without caller identity as described in the October 28 entry.

Next main area: dependency and build security. Start with the Studio minimatch migration, image build contexts, and frozen-install coverage.


## 2026-11-02

Reviewed commit: `f17c702f45d2145b6ecb45bfd1a27e2f3128ae19`.

HEAD has not changed since the November 1 review. The run started with 26 modified tracked files and 10 untracked status entries. Prior runtime, template, dependency, deployment, and workflow fixes were preserved. Those defects are fixed in this working tree. The user then requested a commit and PR for all pending changes. The review branch is `security/daily-review-2026-11-02`. Generated Python cache files are excluded from the commit. No merge or deployment is authorized by that request.

Read SECURITY.md, the October 26 audit, both October 27 reports, the daily log, and the security workflows. Confirmed the existing daily task runs at 09:00 local time. Its next run is November 3. No duplicate task was created.

### Review scope

The main area was dependency and build security. No new exploitable issue was confirmed in the inspected paths. No application code, dependency, or advisory ignore changed during this run. The PR collects the fixes from the prior runs with this new log entry.

Reviewed the root and generated deployment Dockerfiles, image dependency manifests, root Docker exclusions, builder source extraction, build subprocesses, signed artifact upload flow, and frozen-install coverage. The runtime and Studio image installs use frozen locks. The cloud builder permits dependency resolution for pruned workspaces and rewritten workspace dependencies. Generated app Dockerfiles also install without a frozen lock. These paths do not guarantee reproducible dependency resolution. Build source and scripts remain trusted inputs within the isolated build machine. This review did not establish a cross-app exploit. A full review of control-plane isolation remains outside this repository review.

Checked the open Studio minimatch finding against the current advisory and installed callers. ESLint and TypeScript ESLint callers still require minimatch 10. The inspected calls use default optimization settings, not the affected optimization level of 2 or greater. Studio config uses fixed local glob patterns. No production HTTP path to these build tools was found. A major-version override was not applied without compatibility validation.

Rechecked the pending operator, B2B, AI Studio, and production-default changes. Their regression tests passed. Earlier log entries record the before-fix failures. No new regression test was needed because this run adds no source fix.

### Current advisory scans

OSV scanned all 12 tracked supported lockfiles. It returned 1 for findings. `python3 tools/osv-gate.py` returned 0 under the existing severity policy. The scan is not clean.

| Scope | Result |
| --- | --- |
| Production/root Cargo | 9 advisory groups |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build dependency locks | No reported matches in all three locks |
| Benchmark | 8 advisory groups |
| Vendor-only SAML | No reported matches |
| Shard and Swift locks | No reported matches in four locks |

`cargo audit --json` returned 0 with existing ignores. RustSec database commit: `263a4d951a0867d4bddc29dddd6d59427c78f9a7`, updated November 2. It reported zero unsuppressed vulnerabilities, seven unmaintained warnings, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock consistency and advisory-ignore date checks passed. No suppression was added or extended.

Remaining risks:

- Studio minimatch 10.2.5 matches GHSA-g3hv-75wg-m249. The [advisory](https://github.com/advisories/GHSA-g3hv-75wg-m249) lists 11.0.1 as fixed. Exploitation requires attacker-controlled glob patterns and optimization level 2 or greater. Keep build configuration trusted. Upgrade callers or validate a major-version override with Studio lint, tests, and builds. This migration remains open.
- `im 15.1.0` and `sized-chunks 0.6.5` retain memory-safety warnings without fixed releases. Remote exploitation was not established. No validated runtime mitigation was found. Dependency migration needs compatibility tests.
- Seven maintenance warnings and the existing RSA exception remain. Passing checks do not patch these packages.

### Verification

| Command | Final result |
| --- | --- |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `cargo test -p pylon-cli commands::deploy::tests` | 11 passed |
| `cargo test -p pylon-runtime --test admin_mutation_security --test dev_network_access --test bearer_cookie_fallback --test scheduler_new_function` | 17 passed against local test servers |
| `bun test packages/functions/src/ai-studio-template-security.test.ts packages/functions/src/b2b-template-security.test.ts packages/functions/src/member.test.ts packages/functions/src/ssr-og-image.test.ts packages/functions/src/runtime-files.test.ts` | 30 passed |
| `bun run --cwd packages/functions check` and `build` | Both passed |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 passed |
| `cargo check --locked --manifest-path benchmarks/search/Cargo.toml` | Passed |
| `rustfmt --edition 2021 --check crates/cli/src/commands/deploy.rs crates/runtime/tests/admin_mutation_security.rs` | Passed |
| `bash tools/check-image-locks.sh` and `bash tools/check-security-ignores.sh` | Passed |
| `git diff --check` | Passed |

TypeScript checks used the pinned Bun 1.4.4 installed in a temporary directory. Initial advisory scans failed on sandbox cache or DNS restrictions. Reruns with required access completed. The first runtime test failed because the sandbox blocked socket binding. Its rerun passed. Existing Rust compiler warnings remain. Logs and scan JSON are under `/tmp/pylon-security-20261102-*`.

Reviewed the pending source and lockfile diffs before staging. Full workspace tests, Studio lint and build, Windows builds, container builds, live deployments, cloud permissions, system libraries, and Nix flake inputs were not checked. These gaps are not clean results. The root Docker exclusions and cloud builder isolation need further testing with synthetic secret files and a local container service.

### Existing deployments and next review

The PR includes the prior runtime authorization, CSRF, request-size, template ownership, production-default, and dependency fixes. Release the runtime and images through the normal process. Set `PYLON_DEV_MODE=false` on existing deployments and configure trusted origins. Existing B2B and AI Studio apps need their handler changes and regenerated manifests. Review old AI Studio jobs without saved caller identity. Scan each app's own lockfile and apply the Satori and fflate updates. No deployed app changed during this run.

Next main area: authentication and sessions. Check session revocation, token transport, account linking, and generated authentication flows.


## 2026-11-06

Reviewed commit: `5f32359410ee6fe5b04b385b9679bcecc288462e`.

The last successful logged review was November 2 at `f17c702f45d2145b6ecb45bfd1a27e2f3128ae19`. Reviewed the intervening OIDC refresh-token commit and the pending security changes. The run started on `security/daily-review-2026-11-02` with 26 modified tracked files and 10 untracked status entries. Prior fixes and reports were preserved. They are included in the requested commit and PR. Python cache files are excluded. No merge, deployment, or release was made.

Read SECURITY.md, the October 26 audit, both October 27 reports, prior daily entries, and the security workflows. GitHub reports both security workflows as active. The separate local daily review-and-fix task could not be inspected: this session exposes no task scheduling tool or local automation directory. No duplicate task was created. Workflow activity does not establish that an automatic fix task is scheduled.

### Authentication and sessions

Reviewed OIDC client authentication, refresh scope limits, token hashing and storage, revocation, provider-token access gates, account linking, token error redaction, session transport, and generated authentication configuration. Inspected the B2B web cookie helper, B2B API schema, consumer API auth configuration, and AI Studio auth configuration. Prior template authorization fixes remain present. No new template issue was confirmed in these samples.

**High, conditional: an in-flight OIDC refresh could restore a revoked grant.** The refresh route called `accept_refresh_token`, then signed and stored new tokens through `issue_tokens`. Revocation deleted the existing rows between these steps. Issuance did not check whether the grant still existed. A grant is the stored authorization for one user and one client.

The deterministic regression accepts a refresh, revokes the grant, then resumes issuance. It failed before the fix because issuance succeeded. The same sequence applies to user-wide revocation. An attacker needs a valid refresh token, the matching client secret for a confidential client, and a request that overlaps revocation. Public clients do not require a secret. The affected deployment must enable Pylon's OIDC provider and issue `offline_access` grants. The impact is continued account access after token revocation or sign-out everywhere. The test controls the interleaving at the provider API. It does not measure an HTTP attack success rate.

Token issuance now writes the access and refresh tokens as one transaction. A continuing grant must have a live refresh record with the same user and client. Memory uses one mutex. SQLite uses an immediate write transaction. PostgreSQL issuance and both revocation paths use a transaction lock for the user. PostgreSQL explicitly uses read-committed isolation so the check sees a revocation that completed while it waited. No schema change is required. Concurrent refreshes of an active grant remain supported.

The new provider regression passes. Shared store tests cover concurrent grant and user revocation, refusal to restore a deleted grant, and rollback of a partially failed token bundle. SQLite also runs the concurrency check across separate connections. The PostgreSQL tests run against a temporary local PostgreSQL 16 service. No production service was tested.

### Current dependency results

OSV Scanner scanned all 12 tracked supported lockfiles with current advisory data. It returned 1 for findings. `python3 tools/osv-gate.py` returned 0 under the existing severity policy. This is not a clean scan.

| Scope | OSV result |
| --- | --- |
| Production/root Cargo | 9 advisory groups |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build locks | No reported matches in all three locks |
| Search benchmark | 8 advisory groups |
| Vendor-only SAML | No reported matches |
| Shard and Swift locks | No reported matches in four locks |

`cargo audit --json` returned 0 with existing ignores. RustSec database commit: `d1d82c33f65871596a8b2a0a53b86e9b3a730951`, updated November 6. It reported zero unsuppressed vulnerabilities, seven unmaintained warnings, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock consistency and advisory-ignore checks passed. No dependency version or advisory ignore changed during this run. The PR includes the previously prepared dependency updates.

Remaining dependency risks:

- Studio minimatch 10.2.5 still matches [GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249). The current advisory lists 11.0.1 as fixed. Its callers require version 10. Exploitation requires attacker-controlled glob patterns and optimization level 2 or greater. Keep build configuration trusted. A caller upgrade or tested major-version migration remains required. Studio lint was not run here. Studio build passed during PR preparation.
- `im` and `sized-chunks` retain memory-safety warnings. [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) still has no patched release. Remote exploitation was not established. Migration requires compatibility testing. No validated runtime mitigation was found.
- Seven maintenance warnings and the existing RSA exception remain. Passing gates do not patch these packages.

### Verification

| Command | Result |
| --- | --- |
| `cargo test -p pylon-auth --lib refresh_cannot_restore_a_revoked_grant` before fix | Failed as expected |
| `cargo test -p pylon-auth -p pylon-functions --lib` after fix | 499 auth and 93 function tests passed |
| `cargo test -p pylon-runtime --lib oidc_token_backend` | 3 passed |
| `cargo test -p pylon-runtime --test admin_mutation_security --test bearer_cookie_fallback --test dev_network_access --test oidc_provider --test oidc_refresh_tokens` | 29 passed across the five suites |
| `PYLON_TEST_PG_URL=<local test service> cargo test -p pylon-runtime --test postgres_auth_backends -- --test-threads=1` | 11 passed; database paths exercised |
| `cargo test -p pylon-cli commands::deploy::tests` | 11 passed |
| Bun tests: AI Studio, B2B, membership, OG image, runtime files, runtime auth operations | 33 passed |
| `bun run --cwd packages/functions check` and `build` | Both passed with Bun 1.4.4 |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 passed |
| `cargo check --locked --manifest-path benchmarks/search/Cargo.toml` | Passed |
| `cargo fmt --all -- --check` | Passed |
| Image-lock check, ignore-date check, and `git diff --check` | Passed |

The initial PostgreSQL run failed four tests during concurrent schema creation. The database log showed duplicate catalog entries from `CREATE TABLE IF NOT EXISTS`. The sequential suite passed. Its OIDC tests still spawn concurrent operations. Existing compiler warnings remain. Local scan and test evidence uses `/tmp/pylon-security-20261106-*`.

Full workspace tests, Studio lint, Windows builds, container image builds and scans, cloud permissions, deployed secrets, operating-system libraries, and Nix flake inputs were not checked. These are scan and test gaps. The final source diff and dependency diffs were inspected.

### PR preparation on current main

Fetched current `main` at `617adefb6`. The local branch had an OIDC feature commit with a different hash from the upstream copy. Committed the local security changes as `7fc1b709c`. Applied only that security commit to `security/daily-review-2026-11-06` in `/tmp/pylon-security-pr-20261106`. This preserves the original branch and excludes the unrelated feature commit from the PR.

Reviewed the additional main changes: release versions, the Wasmtime update, the wasm32 revocation wrapper, and LLM cache and tool-choice request handling. No additional exploitable issue was confirmed in this diff review. The authentication focus remains the OIDC token lifecycle.

Resolved the Bun lockfile conflict with Bun 1.4.4. Kept main's newer dependency entries. Regenerated the benchmark's local package versions with `cargo update --workspace --manifest-path benchmarks/search/Cargo.toml`. This changed 17 local packages from 0.22.63 to 0.22.69. No additional external package upgrade was introduced.

Repeated the advisory scans in the PR worktree. OSV again scanned all 12 lockfiles and reported the same groups. The OSV gate, Cargo audit, Cargo deny, image-lock check, and ignore-date check passed. Repeated the auth and function unit suites, HTTP suites, PostgreSQL suite, Bun suites, functions type check and build, scanner tests, and scaffold tests. Their pass counts match the table above.

The first runtime build in the worktree failed because Studio's generated `web/dist/index.html` was absent. Installed Studio dependencies from its frozen lockfile with Bun 1.4.4 and built Studio successfully. The runtime HTTP and PostgreSQL reruns passed. A SQLite store test then failed at database initialization with `database is locked`. The test path helper used only the process ID and clock. Added a process-local counter so concurrent tests cannot select the same path. The three SQLite tests then passed. The final CLI deployment suite passed all 11 tests. The locked benchmark build and formatting checks also passed on the PR worktree.

### Deployment follow-up and next review

Upgrade every OIDC provider replica before relying on this fix. Old replicas do not take the new PostgreSQL lock. Revoke affected grants again after all replicas are patched if compromise is suspected. Already issued signed ID tokens retain their normal expiry. Custom Rust implementations of `OidcTokenStore` must implement the atomic `insert_tokens` contract.

Release the prior runtime and image fixes through the normal process. Set `PYLON_DEV_MODE=false` and trusted origins on existing deployments. Existing B2B and AI Studio apps need the prior handler changes and regenerated manifests. Review old AI Studio jobs without saved caller identity. Apply the Satori and fflate updates to independent app dependencies. Scan each app's lockfile. The metrics token still permits the existing privileged read routes; protect it as a data-access credential.

Next main area: authorization and tenant isolation. Include provider-token caller gates and organization-scoped function access.


## 2026-11-07

Reviewed checkout commit: `5595e7676ea49282af7d715907e46bd878bfc881` on `security/daily-review-2026-11-02`.
Reviewed PR worktree commit: `11d4ee1cf` on `security/daily-review-2026-11-06` in `/tmp/pylon-security-pr-20261106`.

Both worktrees started with no tracked edits. Each had untracked Python cache files. These files were preserved and excluded from the commit. Prior fixes are committed in open [PR #80](https://github.com/pylonsync/pylon/pull/80). They are not open findings in this working tree. This run adds its fix and report to that PR. The primary checkout also receives this commit so the next daily run can see the result.

Read SECURITY.md, both October 27 reports, the daily log, and CI and daily security workflows. The scheduled review task already runs daily at 09:00 local time. Its next run is November 8. No duplicate task was created. The SSH fetch failed because no accepted key was available. Authenticated HTTPS fetched main successfully at `617adefb6`. Main and the PR had no new commits since the last successful review. No production service was tested.

### Authorization and tenant isolation

Traced aggregate, graph, lookup, sync snapshot, update, and function authorization paths. Checked provider-token caller gates, strict function policy handling, the membership helper, and B2B template schema and handlers. Prior B2B checks remain present. The B2B row-dependent read policies reject aggregate requests before storage access. No new template defect was confirmed in these paths.

**Confirmed: malformed aggregate filters bypass the active tenant filter.** In `crates/router/src/routes/queries.rs`, the aggregate route inserts the active `orgId` only when `where` is an object. Field validation previously accepted other JSON types. Both SQLite and PostgreSQL aggregate builders ignore a non-object `where` value. The request could therefore omit the tenant restriction.

The local HTTP regression creates one Project row in each of two organizations. Alice belongs to organization A and selects it. The test uses a row-independent read policy that allows signed-in users. A normal count returns 1. Before the fix, `{"count":"*","where":null}` returned 200 with `{"rows":[{"count":2}]}`. The regression failed on that response. This confirms a cross-tenant aggregate result through the active-tenant filter bypass.

This is a conditional defense-in-depth issue. The caller needs access to the aggregate endpoint, an active tenant, and a policy that permits aggregate reads. The entity must have an `orgId` field and data in another tenant. Row-dependent read policies already deny this route. A broad row-independent policy can also permit other read routes, so this finding does not establish a bypass of a restrictive row policy. Aggregate results can include counts, sums, or grouped public values. The HTTP test confirms the count exposure only.

The route now rejects a non-object aggregate specification or `where` with 400 before it applies tenant scope or calls storage. No policy was weakened. Unit tests cover null, arrays, strings, booleans, and numbers. The HTTP regression checks those filters, a missing filter, and an explicit foreign `orgId`. Valid requests still return only the active tenant's count. This validation is shared by both storage backends. The HTTP reproduction used SQLite; PostgreSQL was traced in source but was not run today.

### Current advisory scans

OSV scanned all 12 tracked supported lockfiles in both worktrees. Both scans returned 1 for findings. The severity gate returned 0. These are not clean scans.

| Scope | OSV result in the PR worktree |
| --- | --- |
| Production/root Cargo | 9 advisory groups |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build | No reported matches in three locks |
| Search benchmark | 8 advisory groups |
| Vendor-only SAML | No reported matches |
| Shard and Swift | No reported matches in four locks |

`cargo audit --json` returned 0 with the existing ignores. RustSec database commit: `309392b18f6b17022f82c701f300433126901068`, last updated November 6. It reported zero unsuppressed vulnerabilities, seven unmaintained warnings, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock consistency and advisory-ignore checks passed. No dependency or ignore changed.

Existing risks remain:

- Studio minimatch 10.2.5 matches [GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249). The current advisory lists 11.0.1 as fixed. Exploitation requires attacker-controlled patterns with optimization level 2 or greater. The prior review found version-10 callers and no affected production HTTP path. Keep build configuration trusted. A caller upgrade or tested major-version migration remains required. This run did not validate that migration.
- `im` and `sized-chunks` retain memory-safety warnings. [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) still lists no patched version. Remote exploitation was not established. Dependency migration needs compatibility tests; no validated runtime mitigation was found.
- Seven maintenance warnings and the existing RSA exception remain. Passing checks do not patch them.

### Verification

| Command | Result |
| --- | --- |
| `cargo test -p pylon-runtime --test update_policy_scope aggregate_rejects_invalid_where_without_losing_tenant_scope` before fix | Failed as expected: 200 and count 2 instead of 400 |
| `cargo test -p pylon-runtime --test update_policy_scope --test oauth_tenant_handoff` after fix | 17 passed on local HTTP servers |
| `cargo test -p pylon-router -p pylon-policy -p pylon-functions --lib` after fix | 178 router, 107 policy, and 93 function tests passed; one existing performance test ignored |
| `bun test packages/functions/src/b2b-template-security.test.ts packages/functions/src/member.test.ts packages/functions/src/runtime-auth-ops.test.ts` | 21 passed with pinned Bun 1.4.4 |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `python3 tools/scan-lockfiles.py` | All 12 locks scanned; exit 1 for findings |
| `python3 tools/osv-gate.py` on scan JSON | Exit 0 under the existing severity policy |
| `cargo audit --json` and `cargo deny check` | Exit 0 with the warnings above |
| `bash tools/check-image-locks.sh` and `bash tools/check-security-ignores.sh` | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

The first regression draft had an incorrect expected response envelope. It was corrected before the recorded failure that demonstrated count 2. Rust tests compiled the changed router and runtime. Existing compiler warnings remain. No TypeScript source or dependency changed, so no additional TypeScript build was required. Local evidence uses `/tmp/pylon-security-20261107-*`. The final source and report diff was inspected before commit.

Full workspace tests, PostgreSQL integration tests, Studio lint and build, Windows builds, container builds and image scans, cloud permissions, deployed secrets, operating-system libraries, and Nix flake inputs were not checked today. These are gaps, not clean results.

### Deployment follow-up and next review

Release the runtime fix through the normal process. Existing clients must omit `where` or supply an object; `where: null` now returns 400. Existing generated apps receive this fix through a runtime update. Keep row-dependent tenant policies for private data. Do not rely only on the aggregate route's extra tenant filter.

The prior OIDC, runtime, template, image, and dependency follow-up in the November 6 entry still applies. Update every OIDC replica. Existing B2B and AI Studio apps need their prior handler changes. Configure production mode and trusted origins. Scan each application's lockfile. No merge, deployment, or release was performed.

Next main area: HTTP, CSRF, CORS, and WebSocket boundaries. Include origin checks on early-return routes and tenant changes on persistent connections.


## 2026-11-08

Reviewed primary checkout: `1e817c447` on `security/daily-review-2026-11-02`.
Reviewed PR worktree: `0abb67f37` on `security/daily-review-2026-11-06` in `/tmp/pylon-security-pr-20261106`.

Both worktrees had no tracked edits. Untracked Python cache files were preserved. Read SECURITY.md, both October 27 reports, prior daily entries, and the CI and daily security workflows. Prior fixes remain in open [PR #80](https://github.com/pylonsync/pylon/pull/80). They are not new open findings. Authenticated HTTPS fetch confirmed that main remains at `617adefb6`. No new main or PR source changes required review since November 7. The existing task runs daily at 09:00 local time. Its next run is November 9. No duplicate task was created.

### HTTP, CSRF, CORS, and WebSocket boundaries

Traced early admin returns, the form-route CSRF gate, bearer-to-cookie fallback, CORS origin selection, and the main-port WebSocket cookie origin gate. Checked the dedicated WebSocket explicit-token path, session revocation, tenant changes, role enrichment, and broadcast authorization. Reviewed the B2B server cookie helper and schema, plus consumer and AI Studio auth defaults. These templates use shared runtime controls. No new template or origin-check defect was confirmed in these samples.

**Confirmed, conditional: a connected WebSocket could retain revoked role or admin access.** `WsHub::revalidate` compared only the resolved user and tenant. Broadcast policy checks used the stored connection identity. A role downgrade or removal of database-derived admin status could leave both IDs unchanged. The connection therefore kept its old permissions through repeated credential checks.

An attacker needs an existing authenticated sync connection opened while privileged. The deployment must use organization-role or database-derived admin permissions that are later revoked. Role-based read policies can then continue to allow private updates to that connection. This is continued access after revocation, not anonymous access. Both sync WebSocket entry points share this lifecycle. Row-dependent membership policies and the separate room access refresh already provide their own checks; this finding does not establish a bypass of those controls.

The initial regression failed before the fix: revalidation returned no closed client after the role changed. The final regression covers both role downgrade and admin revocation. It confirms an authorized broadcast before revocation, connection closure, removal of CRDT subscriptions, and no later broadcast. It uses a local in-memory connection and a controlled permission resolver. It does not reproduce an attack through a browser or a live database.

Revalidation now starts from the credential, adds current database permissions, and compares the complete identity. A changed identity closes the connection through the existing cleanup path. This preserves credential-owned roles. No authorization control was weakened. The periodic check still runs every 30 seconds; this fix does not promise immediate revocation. Scheduling and database delays can extend that interval. Revoke the session for urgent removal of local access. Multi-replica revocation timing remains a separate deployment concern.

### Current advisory scans

OSV scanned all 12 tracked supported lockfiles in the PR worktree. It returned 1 for findings. `tools/osv-gate.py` returned 0 under the existing severity policy. This is not a clean scan.

| Scope | Result |
| --- | --- |
| Production/root Cargo | 9 advisory groups |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build | No reported matches in three locks |
| Search benchmark | 8 advisory groups |
| Vendor-only SAML | No reported matches |
| Shard and Swift | No reported matches in four locks |

`cargo audit --json` returned 0 with existing ignores. RustSec database commit: `309392b18f6b17022f82c701f300433126901068`, last updated November 6. It reported zero unsuppressed vulnerabilities, seven unmaintained warnings, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock consistency and advisory-ignore checks passed. No dependency or ignore changed.

Current upstream data still lists minimatch 11.0.1 as the fix for [GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249). The existing Studio version-10 callers need a tested major migration or caller upgrade. Keep build patterns trusted. No production HTTP exploit was established. [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) still lists no patched sized-chunks release. The im and sized-chunks memory-safety warnings, maintenance warnings, and existing RSA exception remain. Migration needs compatibility tests. No validated runtime mitigation was found for the memory-safety warnings. Passing gates do not resolve these risks.

### Verification

All commands ran in the PR worktree unless noted.

| Command | Result |
| --- | --- |
| `cargo test -p pylon-runtime --lib revalidation_closes_a_socket_after_role_or_admin_revocation` before fix | Failed as expected: no closed client |
| `cargo test -p pylon-runtime --lib ws::tests` after fix | 108 passed; this substring also selects workflow tests |
| `cargo test -p pylon-runtime --lib request_auth::` | 7 passed |
| `cargo test -p pylon-runtime --lib server::ws_origin_tests` | 3 passed |
| `cargo test -p pylon-plugin --lib csrf` | 14 passed |
| `cargo test -p pylon-runtime --test admin_mutation_security --test bearer_cookie_fallback --test csrf_form_route --test ws_live_fanout --test oauth_tenant_handoff --test rooms_ws_push` | 21 passed across six local HTTP/WebSocket suites |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 passed |
| `python3 tools/scan-lockfiles.py` | All 12 locks scanned; exit 1 for findings |
| `python3 tools/osv-gate.py` on scan JSON | Exit 0 |
| `cargo audit --json` and `cargo deny check` | Exit 0 with warnings above |
| Image-lock and advisory-ignore checks | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

Rust tests compiled the changed runtime. Existing compiler warnings remain. No TypeScript or dependency changed, so no TypeScript type check or build was run. The final source and report diff was inspected. Local scan and test evidence uses `/tmp/pylon-security-20261108-*`.

Full workspace tests, PostgreSQL tests, Studio lint and build, Windows builds, container builds and image scans, cloud permissions, deployed secrets, operating-system libraries, and Nix flake inputs were not checked today. No production service was tested. These are gaps, not clean results.

### Deployment follow-up and next review

Release the runtime fix to every replica. Existing generated apps receive this fix through a runtime update. No new template migration is required for this finding. Prior OIDC, template, production-mode, and dependency follow-up from November 6 and 7 still applies. Existing B2B and AI Studio apps need those earlier handler changes. Scan each app's lockfile. No merge, deployment, or release was performed.

This run commits the fix and report to PR #80 and copies the commit to the primary checkout. The user's final instruction authorizes the PR push. Untracked cache files remain excluded.

Next main area: files, SSRF, and path traversal. Include remote fetch redirects, uploaded-file authorization, and generated app file handlers.

## 2026-11-09

Reviewed primary checkout: `8248736ea` on `security/daily-review-2026-11-02`.
Reviewed PR worktree: `eaa3ad352` on `security/daily-review-2026-11-06` in `/tmp/pylon-security-pr-20261106`.

Both worktrees started without tracked edits. Untracked Python caches were preserved. Read SECURITY.md, both October 27 reports, prior daily entries, and the CI and daily security workflows. Fetched origin. Main remains at `617adefb6`. The only PR commit since the previous reviewed base is the November 8 WebSocket fix. It was checked as an existing fix. No new main change required review. The daily task remains set for 09:00 local time. Its next run is November 10. No duplicate task was created.

### Files, server-side fetches, and path traversal

Traced upload initialization, confirmation, local PUT, file deletion, and download authorization. Checked local and private S3 owner records, active tenant checks, public visibility, signed URL expiry, and file response types. Reviewed server-function file hooks and the requirement for application authorization before minting a signed URL. Checked local ID validation and canonical paths for static files and local image sources. Reviewed remote image host/path checks, redirect revalidation, the guarded DNS resolver, byte limits, and decoded pixel limits. Sampled consumer image URL validation and upload visibility, backend consumer handlers, and the AI Studio fetch targets. No new application bypass was confirmed in these paths.

### Confirmed dependency issue and fix

Current RustSec data added `RUSTSEC-2026-0459` to today's results. The root and search benchmark locks used `libsqlite3-sys 0.33.0` through `rusqlite 0.35.0`, with bundled SQLite enabled. Cargo audit and Cargo deny both failed before the fix. OSV reported the issue at CVSS 6.8; its existing high-severity gate passed despite this finding.

The [upstream SQLite fix](https://github.com/sqlite/sqlite/commit/e5bf119821f25be1d7f10ef0a2edbe175289166e) initializes the rest of a page when its WAL frame is shorter than the database page. WAL means write-ahead log. Without the fix, a read can retain old buffer bytes. An attacker must supply or modify the database and its sibling WAL file, then have the victim open them. Pylon opens SQLite files for application storage, sessions, jobs, and CLI migration. The CLI restore command can copy both database and WAL files. This confirms an affected dependency and relevant file-open paths. It does not establish anonymous HTTP exploitation. Use trusted backup files and restrict write access to database directories until updated.

Updated the workspace to `rusqlite 0.41.1` and `libsqlite3-sys 0.39.1`, which bundles SQLite 3.54.1. Regenerated the root and benchmark locks with Cargo 1.98.1. Docker uses Rust 1.98 and the workspace manifest and lockfile. The dependency's Rust 1.89 minimum is within that toolchain. Docker JavaScript dependency consistency passed. No advisory ignore changed.

Reviewed the dependency API changes. Enabled `fallible_uint` to preserve checked `u64` and `usize` conversions. Updated three existing SQL-trace tests to the new `trace_v2` API and checked its result. Expanded SQL remains available to the query assertions. The existing column-name iterator usage compiles unchanged. Lockfile changes also include the new hashlink dependency, target-specific SQLite WASM packages, and Cargo's Windows dependency resolution changes. Windows and WASM targets were not built.

Added `tools/sqlite-wal-regression.c`. This bounded regression uses the actual SQLite internal WAL reader with a controlled file reader. It supplies a 512-byte frame to a 4096-byte buffer filled with known bytes. It checks the frame contents and requires the remaining bytes to be zero. It failed with exit 1 against the old bundled source and passed against the new source. No real database or process secrets were read. This tests the defective read operation directly; it is not an end-to-end exploit against a Pylon server. The file includes compilation instructions. It is a manual regression, not a new CI job.

### Current advisory scans

Final OSV scan covered all 12 tracked supported lockfiles. Exit 1 reports remaining findings. The OSV severity gate returned 0. The scan is not clean.

| Scope | Final result |
| --- | --- |
| Production/root Cargo | 9 advisory groups; SQLite match removed |
| Root Bun | No reported matches |
| Studio build tools | 1 group: minimatch 10.2.5 |
| Docker/build | No reported matches in three locks |
| Search benchmark | 8 advisory groups; SQLite match removed |
| Vendor-only SAML | No reported matches |
| Shard and Swift | No reported matches in four locks |

Final `cargo audit --json` returned 0 with existing ignores. RustSec commit: `899077d9d471b9fd3dbf4b2faea8ef26e83692e8`, updated November 9. It reported zero unsuppressed vulnerabilities, seven maintenance warnings, and two unsoundness warnings. `cargo deny check` passed advisories, bans, licenses, and sources. Image-lock and ignore-date checks passed.

Existing risks remain. [Minimatch GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249) still lists 11.0.1 as fixed. Studio's version-10 callers need a tested migration. Keep build patterns trusted. No affected production HTTP path was established. The im and sized-chunks memory-safety warnings remain; [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) has no patched release. Migration needs compatibility tests. No validated runtime mitigation was found. The RSA exception and maintenance warnings also remain. Passing gates do not resolve them.

### Verification

Commands ran in the PR worktree. Initial scans also covered the primary checkout.

| Command | Result |
| --- | --- |
| Compile and run `tools/sqlite-wal-regression.c` against old/new bundled sources | Old failed as expected; new passed |
| `cargo test -p pylon-storage --lib` | 208 passed |
| `cargo test -p pylon-runtime --lib` | 1,185 passed; 7 existing tests ignored |
| `cargo test -p pylon-runtime --test file_serving --test functions_files --test vector_search --test update_policy_scope --test oidc_refresh_tokens --test admin_mutation_security` | 45 passed |
| `cargo test -p pylon-runtime --test functions_files` with Bun 1.4.4 on PATH | Passed; repeats the function file integration test with pinned Bun |
| `cargo test -p pylon-cli commands::migrate` | 10 passed |
| `cargo test -p pylon-kernel net_guard` | 6 passed |
| `cargo check --locked --manifest-path benchmarks/search/Cargo.toml` | Passed |
| Bun 1.4.4 tests: runtime-files, AI Studio security, consumer social | 20 passed |
| `python3 -m unittest discover -s tools -p 'test_*.py'` | 3 passed |
| `node --test packages/create-pylon/test/scaffold.test.mjs` | 10 passed |
| `python3 tools/scan-lockfiles.py` | All 12 locks scanned; exit 1 for findings |
| `python3 tools/osv-gate.py` on final JSON | Exit 0 |
| Cargo audit and Cargo deny | Exit 0 after update |
| Image-lock and advisory-ignore checks | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

Initial upgrade builds failed on the removed trace API and the newly optional integer conversions. Follow-up compilation caught the trace callback lifetime and optional expanded SQL result. These were corrected before the passing runs. No test assertion or authorization control was removed. The first HTTP suite used the installed Bun 1.3.14; the file-function test was repeated with pinned Bun 1.4.4. Existing compiler warnings remain. Local evidence uses `/tmp/pylon-security-20261109-*`.

The final source, lockfile, regression, and report diffs were inspected. Full workspace tests, PostgreSQL integration tests, Windows/WASM builds, Studio lint/build, container builds and image scans, deployed secrets, cloud permissions, system libraries, and Nix flake inputs were not checked. No TypeScript source changed; no TypeScript type check or build was run. These are gaps, not clean results. No production service was tested.

### Deployment follow-up and next review

Rebuild and release the runtime, CLI, and Docker images with the updated lockfile. Existing generated apps need that runtime update. This finding requires no template handler migration. Restrict database directory access and accept only trusted backups. Existing B2B and AI Studio apps still need the earlier handler changes. The prior OIDC replica, production-mode, and independent app dependency follow-up remains in force. Scan each application's own lockfile.

This run adds its commit to PR #80 and copies it to the primary checkout. The final user instruction authorizes the PR push. No merge, deployment, or release is performed. Python cache files remain excluded.

Next main area: server functions and generated app templates. Include signed file URL authorization at application call sites and job caller identity.
