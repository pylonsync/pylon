Security fixes — 2026-10-27

These changes address the application findings in `SECURITY_REVIEW_2026-10-27.md`. They are local changes. They have not been released or deployed.

**Application fixes**

- Operator management and Studio ticket issuance now require an admin token or an authorized admin session. The metrics token remains valid on the existing read routes.
- Cookie-authenticated mutations on those routes now use the shared CSRF check. A valid explicit admin bearer can authorize a server request. An invalid or metrics bearer cannot bypass the cookie origin check.
- Operator POST requests require application/json. Their bodies have a 64 KiB limit. The limit applies to Content-Length and chunked requests. Invalid JSON returns 400. Unsupported content types return 415. Oversized bodies return 413.
- B2B template functions now check the caller's organization membership. Member changes and project archiving require an owner or admin role. Ordinary members can read only their own membership row. They can read and create projects in their organization.
- Organization creation writes its initial owner membership through an explicit server-authorized operation. The organization was created for the caller in the same mutation. This permits initial membership creation when strict function policies are enabled.

**Dependency and scan changes**

- Satori is updated to 0.33.5. Root overrides also cover the older published functions packages used by the site.
- fflate is fixed at 0.8.3 in the root and Docker framework dependency sets. Satori's own dependency still requests 0.7.3, so the override is required.
- block-buffer 0.12.0 is updated to 0.12.1.
- The yanked crypto-bigint 0.7.7 release is updated to 0.7.10.
- The standalone SAML lockfile now resolves OpenSSL 0.10.81, quick-xml 0.41.0, and rand 0.9.5.
- The search benchmark lockfile now resolves current runtime dependencies. It was too old to describe the current manifest. This accounts for its large diff. The benchmark now uses the same local SAML and tiny_http patches as the main workspace. Its manifest initializers were updated for the current runtime types.
- CI and the daily scan use `tools/scan-lockfiles.py`. This script passes every tracked lockfile with a recognized name to OSV. The current repository has 12 such files. These include Docker, benchmark, vendor, shard, and Swift lockfiles. The Nix `vendor/samael/flake.lock` file is outside this OSV parser list. Nix inputs need separate tooling. Untracked build and dependency directories are not scan inputs.
- Scan errors remain failures. Findings still pass through the existing severity policy. No advisory ignore was added.

The root and Docker framework installs were regenerated with Bun 1.4.4. The image manifest consistency check passes.

**Verification**

The new HTTP regression fails against the original server because a metrics token receives 201 when it must receive 401. It passes with the fix. The B2B tests have six failures against the original handlers and pass with the fixes.

The final runs passed 481 authentication tests, 44 runtime regression tests, 23 TypeScript tests, 10 scaffold tests, and 3 scanner tests. The functions type check and benchmark build passed. Cargo audit and cargo-deny passed with the existing warning policy. OSV returned findings; the existing severity gate passed. The dependency scan is not clean.

The selected checks cover:

- Production-mode operator credentials, Studio tickets, forged origins, cookie fallback, password preservation, malformed JSON, and both body-length paths.
- Existing Studio access, bearer/cookie fallback, tenant and row policies, uploaded-file handling, and form-route CSRF controls.
- B2B outsiders, ordinary members, administrators, foreign organization members, and organization creation.
- Satori image rendering and PNG route responses.
- Lockfile enumeration and scanner error propagation.
- Functions TypeScript checks, template scaffold tests, and the search benchmark build.

The B2B tests run the actual handlers with the real membership helper and an unfiltered test database. They do not run a complete generated app over HTTP. The HTTP server tests use memory-only stores. No production service or container image was tested.

**Remaining dependency findings**

The expanded OSV scan still reports Rust dependency warnings in the root and benchmark lockfiles. It reports no matches in the scanned Bun, Swift, shard, or standalone SAML lockfiles after these updates.

`block-buffer 0.10.4` remains a scanner match. The fixed 0.12.1 release cannot replace a 0.10 dependency through a compatible lockfile update. The inspected 0.10.4 source stores its position separately from the data buffer, unlike the inline-position failure described by the advisory. This review did not establish remote exploitation. The match remains visible pending upstream clarification or dependency migration.

`im` and `sized-chunks` still have memory-safety warnings without fixed releases in the current dependency chain. Maintenance warnings also remain. The existing RSA exception remains in place. These warnings are not fixed by a successful Cargo audit exit code. No new suppression hides them.

**Deployment steps**

Build and release the patched runtime and Docker images. Update running apps to that release. Existing apps generated from the B2B template must receive the handler changes; updating the scaffolder does not edit an existing app.

Regenerate and scan each application's own lockfile. Root package overrides do not propagate to consumers of a published library. Apps that install these dependencies independently must also select patched Satori and fflate versions.

The metrics token still permits the existing privileged read endpoints. Protect it as a data-access credential. If an exposed token may have been used, revoke it and inspect Studio operators and sessions. The code fix does not remove previously created attacker accounts.
