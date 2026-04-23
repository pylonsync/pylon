# Releasing Pylon

Tag-driven release. Push `vX.Y.Z` → the `release.yml` workflow publishes:

1. npm packages (`@pylonsync/sdk`, `/functions`, `/react`, `/sync`)
2. crates.io (every publishable workspace member, topologically ordered)
3. Prebuilt binaries (macOS arm64/x64, Linux x64/arm64/musl, Windows x64)
4. Docker image (`ghcr.io/pylonsync/pylon:X.Y.Z`)
5. GitHub Release with auto-generated notes

## One-time setup

### GitHub repo secrets

Settings → Secrets and variables → Actions → New repository secret:

| Secret | Where to get it |
|---|---|
| `NPM_TOKEN` | npmjs.com → Account → Access Tokens → **Granular**, scope to the `@pylonsync` org, write-only. |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → scope `publish-update`. |

`GITHUB_TOKEN` is injected automatically; no manual setup.

### Verify name availability (first publish only)

Every name we want on crates.io must be unclaimed:

```bash
for name in pylon pylon-cli pylon-kernel pylon-schema pylon-query pylon-storage \
            pylon-policy pylon-auth pylon-plugin pylon-http pylon-router \
            pylon-runtime pylon-sync pylon-staticgen pylon-migrate \
            pylon-action pylon-functions pylon-workers; do
  status=$(curl -sI "https://crates.io/api/v1/crates/$name" | head -1)
  echo "$name → $status"
done
```

A `200` means the name is taken; a `404` means it's free. If any are squatted, rename the corresponding crate in `crates/<name>/Cargo.toml` **before** the first tag.

### Required metadata (crates.io rejects without it)

The workspace `[workspace.package]` table in root `Cargo.toml` must include:

```toml
[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
authors = ["Eric Campbell <eric@pylonsync.com>"]
description = "Pylon — realtime backend framework"
repository = "https://github.com/pylonsync/pylon"
homepage = "https://pylonsync.com"
readme = "README.md"
keywords = ["realtime", "database", "sync", "backend"]
categories = ["database", "web-programming"]
```

Each per-crate `Cargo.toml` should opt-in:

```toml
[package]
name = "pylon-kernel"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Pylon manifest types and diagnostics (short, per-crate description)"
repository.workspace = true
homepage.workspace = true
readme = "../../README.md"
keywords.workspace = true
categories.workspace = true
```

**Path dependencies must also carry a `version`** or cargo publish refuses:

```toml
# Wrong — cargo publish will reject
pylon-kernel = { path = "../core" }

# Right
pylon-kernel = { path = "../core", version = "0.1.0" }
```

Or use workspace dep inheritance in root `Cargo.toml`:

```toml
[workspace.dependencies]
pylon-kernel = { path = "crates/core", version = "0.1.0" }
pylon-storage = { path = "crates/storage", version = "0.1.0" }
# … one line per internal crate …
```

Then each crate just says `pylon-kernel.workspace = true`.

### Required npm metadata

Each published `package.json` needs:

```json
{
  "name": "@pylonsync/sdk",
  "version": "0.1.0",
  "description": "Pylon schema DSL — entity, field, policy, buildManifest",
  "license": "MIT OR Apache-2.0",
  "repository": { "type": "git", "url": "git+https://github.com/pylonsync/pylon.git", "directory": "packages/sdk" },
  "homepage": "https://pylonsync.com",
  "bugs": "https://github.com/pylonsync/pylon/issues",
  "keywords": ["pylon", "realtime", "database"]
}
```

## Cutting a release

1. Bump the version everywhere:

   ```bash
   # Rust workspace (single source)
   sed -i '' 's/^version = ".*"/version = "0.2.0"/' Cargo.toml

   # npm packages
   for pkg in packages/*/package.json; do
     node -e "const p=require('./$pkg'); p.version='0.2.0'; require('fs').writeFileSync('./$pkg', JSON.stringify(p, null, 2) + '\n')"
   done
   ```

2. Commit + tag + push:

   ```bash
   git add -A
   git commit -m "chore: release v0.2.0"
   git tag v0.2.0
   git push origin main --tags
   ```

3. Watch the workflow at `github.com/pylonsync/pylon/actions`. Jobs run in this order:

   - `verify` — fails fast if tag ≠ Cargo/npm versions
   - `publish-npm` + `publish-crates` + `binaries` — parallel
   - `docker`, `github-release` — after prior jobs

4. Verify the release:

   ```bash
   npm view @pylonsync/sdk version
   cargo search pylon-cli
   gh release view v0.2.0
   ```

## Re-running a failed release

Both `publish-npm` and `publish-crates` are idempotent — they skip any `name@version` already on the registry. Re-run the workflow from the Actions tab after fixing the root cause.

If a crate *did* publish and needs a correction, you cannot overwrite — bump to the next patch version and re-tag. crates.io does not allow re-publishing the same version.

## Manual dry-run

Before the first real release, verify locally:

```bash
# npm dry-run
for pkg in packages/sdk packages/functions packages/react packages/sync; do
  (cd "$pkg" && npm publish --dry-run --access public)
done

# cargo dry-run (one crate at a time, leaves first)
cargo publish -p pylon-kernel --dry-run
cargo publish -p pylon-schema --dry-run
# ... etc
```

Both dry-runs verify metadata + packaging without touching the registry.
