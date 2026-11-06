Security audit — 2026-10-26

Reviewed commit: `7d30953c12dbcd222b2fcd38b38f54bb8df9850d`.

This audit covers selected server authorization paths, cookie and CSRF controls, request limits, dependency lockfiles, and scan configuration. CSRF means cross-site request forgery. This is a source review and dependency audit. It is not a full penetration test. No live deployment was tested. No application code or dependencies were changed.

The first two findings have high priority. Both can give an attacker a Studio operator account under the stated conditions. The severity ratings below are review judgments, not CVSS scores.

**1. High: the metrics token can manage Studio operators**

Evidence: `crates/runtime/src/server.rs:5465`, `:5515`, `:5530`, and `:10383`.

The `/admin/operators` routes call `verify_admin_or_metrics_auth`. That helper accepts either `PYLON_ADMIN_TOKEN` or `PYLON_METRICS_TOKEN`. The routes then permit operator creation, password changes, and deletion. The error message says that these routes require the admin token or an admin session. Other comments describe the metrics token as a credential for read-only monitoring routes.

A caller with the metrics token can submit `POST /admin/operators` with a new username and password. The caller can then sign in as that operator. The operator has deployment administration access. This requires a configured metrics token and access to the endpoint. It does not require an existing admin session.

Fix: use a separate authorization check for operator management. Accept only the admin token or an authorized admin/operator session. Keep metrics-token access on the intended read-only routes.

Required regression check: a metrics bearer must receive 401 or 403 for operator creation, password changes, and deletion. An authorized admin must still succeed. Source control flow confirms the issue. An HTTP reproduction was not run.

**2. High, conditional: operator mutations bypass CSRF validation**

Evidence: `crates/runtime/src/server.rs:5465`, `:5492`, `:10412`, and `:6240`; `crates/auth/src/cookie.rs:80`.

The same helper accepts an admin or operator cookie without checking Origin or Referer. The operator route returns before the normal dispatcher calls `csrf_check`. Its JSON reader does not require an application/json Content-Type.

An attacker-controlled page can submit JSON with a simple text/plain POST. This can create an operator or change an operator password if the browser sends an authorized cookie. The default SameSite=Lax cookie limits cross-site attacks. It does not block requests from an attacker-controlled origin on the same site, such as a sibling subdomain under the same registrable domain and scheme. SameSite=None also permits cross-site cookie delivery where browser policy allows it. Host-only cookies do not prevent the browser from attaching them to a request addressed to their host.

CORS response headers do not prevent a simple POST from changing state. The CORS code selects a response header; it does not reject this request before the operator handler.

Fix: apply the common CSRF check before every state-changing route that accepts a cookie. Preserve explicit bearer authentication for server clients. Reject unsupported Content-Type values as a separate control.

Required regression check: a POST with a valid operator cookie, an untrusted Origin, and a text/plain JSON body must fail without changing the account store. Test both create and password-change routes. This finding is based on source flow and browser request rules. Browser exploitation was not tested.

**3. Low: operator request bodies have no size limit**

Evidence: `crates/runtime/src/server.rs:5492`.

The operator JSON reader uses `read_to_end` with no byte cap. The later general body limit does not apply to this route. A caller accepted by its authorization check can force a large allocation. This is an authenticated denial-of-service risk. Its significance increases when combined with findings 1 or 2.

Fix: use the common bounded body reader. Reject oversized declared and streamed bodies with 413. Test a body without Content-Length as well as a declared oversized body.

**4. Medium process gap: CI scans only three lockfiles**

Evidence: `.github/workflows/ci.yml:488` and `.github/workflows/security-daily.yml:87`.

The source scans explicitly list the root Cargo lockfile, root Bun lockfile, and Studio Bun lockfile. The recursive audit found affected versions in `benchmarks/search/Cargo.lock` and `vendor/samael/Cargo.lock`. Three Docker Bun lockfiles are also outside those source-scan lists. Image scans provide separate coverage, but they do not make these source-scan lists complete.

Fix: enumerate all tracked supported lockfiles. Classify results by production, build, benchmark, and vendor-only use. If a lockfile is deliberately excluded, record the reason and review date. Do not classify a vendor-local lockfile finding as a production binary finding without checking the workspace resolution.

**Dependency results**

Commands run:

```sh
cargo audit --json
osv-scanner scan source -r --no-resolve --format json --output-file /tmp/pylon-security-audit/osv.json .
cargo deny --locked --offline check advisories
cargo tree --locked --offline -i im -e normal
```

Cargo audit used RustSec database commit `3279c99911354c400e1d41e47c7d60e39258b147`. Its reported update time was 2026-10-26. It returned exit 0 with no unsuppressed vulnerability findings. It also reported seven unmaintained packages and two unsoundness warnings. Cargo deny returned exit 0. OSV returned exit 1 because it found advisories. The configured RSA ignore was applied. These successful Cargo exit codes do not mean that all dependency risk is resolved.

All five scanned Bun lockfiles had no advisory matches: the root, Studio, and three Docker lockfiles. This is a result from the available advisory database. It does not prove that the packages have no vulnerabilities. The recursive scan also inspected the tracked shard lockfiles, Swift resolution files, and C# test manifests. Transitive resolution for manifests without lockfiles was disabled.

Recent TLS disclosures and exact versions:

| Package | Main lockfile | Benchmark lockfile | Advisory and fix |
| --- | --- | --- | --- |
| rustls | 0.23.46 | 0.23.39 | [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html), September 14: handshake encryption boundaries; fixed in 0.23.45. |
| rustls | 0.23.46 | 0.23.39 | [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html), October 14: session Debug output can reveal TLS secrets; fixed in 0.23.46. Requires affected custom logging. |
| rustls | 0.23.46 | 0.23.39 | [RUSTSEC-2026-0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html), October 14: large Brotli allocation before authentication; fixed in 0.23.46. Requires the optional Brotli feature and affected connection configuration. |
| rustls | 0.23.46 | 0.23.39 | [RUSTSEC-2026-0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html), October 14: cached key-exchange downgrade; fixed in 0.23.46. Impact depends on client and server settings. |
| rustls-webpki | 0.103.16 | 0.103.13 | [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html), October 20: accepts distrusted intermediates; fixed in 0.103.16. |
| webpki-roots | 1.0.10 plus 0.26.11 wrapper | 1.0.7 | [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381.html), issued October 20: unsafe trust roots; fixed in 1.0.10. |

The main lockfile contains these fixes. Update the separate benchmark lockfile. A version match does not establish that every affected feature is used by the benchmark.

The `webpki-roots` ignore is consistent with the current advisory: 0.26.11 re-exports the separately resolved 1.x roots. The root lockfile resolves 1.0.10. The current advisory explicitly exempts the wrapper version, so the old ignore may no longer be needed.

The main runtime still includes `loro-internal 1.11.1 -> im 15.1.0 -> sized-chunks 0.6.5`:

- [RUSTSEC-2023-0126](https://rustsec.org/advisories/RUSTSEC-2023-0126.html): undefined behavior during OrdSet insertion. No fix is available. The inspected Loro production use is an im::HashMap. No OrdSet use was found in that source. Remote exploitability was not established.
- [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html), issued August 12: panic during element destruction can cause a double-free or use-after-free. No fix is available. The required panic and affected operation were not demonstrated in Pylon.

Track these warnings with Loro upstream. Review affected call paths before assigning exploit severity. Prefer migration to maintained dependencies when supported. The current cargo-deny policy limits unsoundness enforcement to workspace dependencies, so these transitive warnings do not block it.

`rsa 0.9.10` remains covered by the ignored [Marvin advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html). Upstream still lists no fixed version. Pylon's inspected use is OIDC signing through RandomizedSigner. The configured exception cites the absence of an RSA decryption oracle. Keep this exception under review; do not describe the dependency as patched.

The seven unmaintained packages reported by cargo audit are atomic-polyfill, bincode, bitmaps, im, paste, rustls-pemfile, and sized-chunks. These are maintenance findings, not seven confirmed exploits.

The vendor-local lockfile also contains openssl 0.10.72, quick-xml 0.37.5, and rand 0.9.1 with advisory matches. The root workspace resolves openssl 0.10.81 and quick-xml 0.41.0. The vendor-local results therefore do not establish that these old versions ship in Pylon. Update the vendor lockfile if it is used for standalone tests or builds.

The repository runtime-watch script also ran against current upstream release data. It returned no entries in its `behind` list. It reported Rust 1.98 behind stable 1.99.1 as an informational update. This is a version-watch result, not a scan of deployed images.

**Limits and follow-up**

The audit did not scan running container images or the operating-system libraries linked into them. It did not inspect cloud permissions, deployed environment values, production secrets, or every application endpoint. It did not run the full test suite or an exploit harness. Dependency reports do not cover arbitrary application logic or all native libraries.

Fix findings 1 and 2 first. Add HTTP regression tests before release. Then cap operator request bodies, update the separate lockfiles, and extend dependency-scan coverage. Review the transitive memory-safety warnings with Loro upstream.

Raw scan evidence is in `/tmp/pylon-security-audit/`. This temporary directory is local and is not part of the repository.

**Complete OSV match list**

Aliases can describe the same underlying issue. Each row below is an advisory identifier returned by the scanner.

| Lockfile | Package | Advisory |
| --- | --- | --- |
| Cargo.lock | atomic-polyfill 1.0.3 | [RUSTSEC-2023-0089](https://rustsec.org/advisories/RUSTSEC-2023-0089.html) |
| Cargo.lock | bincode 1.3.3 | [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141.html) |
| Cargo.lock | bitmaps 2.1.0 | [RUSTSEC-2026-0247](https://rustsec.org/advisories/RUSTSEC-2026-0247.html) |
| Cargo.lock | im 15.1.0 | [RUSTSEC-2023-0126](https://rustsec.org/advisories/RUSTSEC-2023-0126.html) |
| Cargo.lock | im 15.1.0 | [RUSTSEC-2026-0248](https://rustsec.org/advisories/RUSTSEC-2026-0248.html) |
| Cargo.lock | paste 1.0.15 | [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436.html) |
| Cargo.lock | rustls-pemfile 2.2.0 | [RUSTSEC-2025-0134](https://rustsec.org/advisories/RUSTSEC-2025-0134.html) |
| Cargo.lock | sized-chunks 0.6.5 | [RUSTSEC-2026-0251](https://rustsec.org/advisories/RUSTSEC-2026-0251.html) |
| Cargo.lock | sized-chunks 0.6.5 | [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html) |
| benchmarks/search/Cargo.lock | rustls 0.23.39 | [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html) |
| benchmarks/search/Cargo.lock | rustls 0.23.39 | [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html) |
| benchmarks/search/Cargo.lock | rustls 0.23.39 | [RUSTSEC-2026-0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html) |
| benchmarks/search/Cargo.lock | rustls 0.23.39 | [RUSTSEC-2026-0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html) |
| benchmarks/search/Cargo.lock | rustls 0.23.39 | [GHSA-2mjx-qc3c-rqvc](https://github.com/advisories/GHSA-2mjx-qc3c-rqvc) |
| benchmarks/search/Cargo.lock | rustls-webpki 0.103.13 | [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) |
| benchmarks/search/Cargo.lock | webpki-roots 1.0.7 | [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381.html) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-8c75-8mhr-p7r9](https://github.com/advisories/GHSA-8c75-8mhr-p7r9) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-ghm9-cr32-g9qj](https://github.com/advisories/GHSA-ghm9-cr32-g9qj) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-hppc-g8h3-xhp3](https://github.com/advisories/GHSA-hppc-g8h3-xhp3) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-phqj-4mhp-q6mq](https://github.com/advisories/GHSA-phqj-4mhp-q6mq) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-pqf5-4pqq-29f5](https://github.com/advisories/GHSA-pqf5-4pqq-29f5) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-xmgf-hq76-4vx2](https://github.com/advisories/GHSA-xmgf-hq76-4vx2) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-xp3w-r5p5-63rr](https://github.com/advisories/GHSA-xp3w-r5p5-63rr) |
| vendor/samael/Cargo.lock | openssl 0.10.72 | [GHSA-xv59-967r-8726](https://github.com/advisories/GHSA-xv59-967r-8726) |
| vendor/samael/Cargo.lock | quick-xml 0.37.5 | [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194.html) |
| vendor/samael/Cargo.lock | quick-xml 0.37.5 | [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195.html) |
| vendor/samael/Cargo.lock | rand 0.9.1 | [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html) |
| vendor/samael/Cargo.lock | rand 0.9.1 | [GHSA-cq8v-f236-94qc](https://github.com/advisories/GHSA-cq8v-f236-94qc) |
