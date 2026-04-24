# Releasing Pylon

Releases are automated via [release-please](https://github.com/googleapis/release-please). You never run `git tag` by hand.

## The flow

```
conventional commit on main
        ↓
release-please opens/updates
"chore: release X.Y.Z" PR  ←— accumulates every commit since last release
        ↓
maintainer merges the PR
        ↓
release-please writes version bumps, pushes git tag vX.Y.Z, creates GitHub Release
        ↓
release.yml fires on the tag and publishes:
  • @pylonsync/* to npm
  • pylon-* crates to crates.io
  • prebuilt binaries (macOS arm64/x64, Linux x64/arm64/musl, Windows x64)
  • Docker image to ghcr.io
```

## Writing commits so release-please understands them

Use [Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Example | Version bump |
|---|---|---|
| `feat:` | `feat(auth): add magic-link flow` | minor (`0.1.0` → `0.2.0`) |
| `fix:` | `fix(sync): race in subscription fan-out` | patch (`0.1.0` → `0.1.1`) |
| `feat!:` or `BREAKING CHANGE:` | `feat!: rename @pylon/* → @pylonsync/*` | major once past `1.0.0`; minor until then |
| `chore:`, `docs:`, `ci:`, `test:`, `refactor:`, `build:`, `style:` | `chore: bump deps` | no release |

The part after the colon is what shows up in the changelog. Make it sentence-case and user-facing — "add X", "fix Y", not "added" or "fixed".

## One-time setup

### GitHub repo secrets

Settings → Secrets and variables → Actions → New repository secret:

| Secret | Where to get it |
|---|---|
| `NPM_TOKEN` | npmjs.com → Account → Access Tokens → **Granular**, scope to the `@pylonsync` org, Publish permission |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → scope `publish-update` |

`GITHUB_TOKEN` is injected automatically; no manual setup.

### Allow the release-please bot to open PRs

Settings → Actions → General → Workflow permissions:

- **Read and write permissions** (required so release-please can push the release commit)
- **Allow GitHub Actions to create and approve pull requests** ✓

### Verify crates.io names are unclaimed (first release only)

```bash
for name in pylon pylon-cli pylon-kernel pylon-schema pylon-query pylon-storage \
            pylon-policy pylon-auth pylon-plugin pylon-http pylon-router \
            pylon-runtime pylon-sync pylon-staticgen pylon-migrate \
            pylon-action pylon-functions pylon-workers; do
  status=$(curl -sI "https://crates.io/api/v1/crates/$name" | head -1)
  echo "$name → $status"
done
```

A `200` means the name is taken; rename the crate in `crates/<name>/Cargo.toml` before the first tag.

## Cutting a release

Nothing to do except merge the release PR when you're ready.

release-please keeps one PR open at all times with the changelog preview. As more `feat:` / `fix:` commits land, it updates the PR. Review → merge → everything else is automatic.

If you want to force a release right now (to ship a hotfix without waiting for more commits), just merge the current release PR — release-please will compute the version from whatever is queued up.

## Troubleshooting

**No release PR appeared after a commit** — check the `release-please` workflow run in Actions. Most common cause: commit didn't match a conventional-commits prefix. Reword the commit (or use `git commit --amend` if it's the last one) and push.

**Release PR shows the wrong version** — release-please infers the bump from commit types. Check that your `feat:`/`fix:` commits use the exact prefix (no `feature:`, no capitalization).

**Publish failed after the tag was created** — both `publish-npm` and `publish-crates` are idempotent. Fix the underlying issue and re-run the `release.yml` workflow from the Actions UI. Crates already published stay; missing ones get retried.

**Need to re-release the same version** — you can't. Bump to the next patch (`fix: re-publish after infra issue`) and merge.

## Manual override

If the automation is broken, you can fall back to manual tagging:

```bash
git tag v0.2.0
git push origin v0.2.0
```

`release.yml` fires the same way. Use sparingly — it bypasses the version-match sanity check in release-please's PR, so you'd need to have already bumped `Cargo.toml` and every `packages/*/package.json`.
