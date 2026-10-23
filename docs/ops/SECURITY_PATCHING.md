# Security patching runbook

How a vulnerability in a dependency, Bun, Node or a base image gets from an
advisory to every running app. Deadlines are in `SECURITY.md`: critical in
24 hours, high in 72 hours.

## 1. Triage

1. Find where the package is used:
   - Rust: `cargo tree -i <crate> -e normal`
   - npm: `grep -n '"<package>@' bun.lock crates/studio_api/web/bun.lock`
   - Images: `trivy image --severity HIGH,CRITICAL --ignore-unfixed ghcr.io/pylonsync/pylon:<version>`
2. Decide whether it applies. A test-only or build-only package still gets
   fixed, but it does not need a fleet move.
3. If there is no fix, or the code path is unreachable, add an ignore with a
   reason and a review date (see `SECURITY.md`) and stop here.

## 2. Fix on main

- Crate: `cargo update -p <crate>` (or `-p <crate>@<old version>` when two
  versions are in the tree). Then `cargo deny check` and `cargo audit`.
- npm: `bun update <package>` in the directory that declares it. For a
  transitive package, add it to `overrides` and `resolutions` in the root
  `package.json`. Then `bun install` and the package's tests.
- Bun: change the `oven/bun` tag and digest in `Dockerfile` and
  `builder/Dockerfile`, and `bun-version` in `.github/workflows/*.yml`.
- Debian packages: move the `debian:trixie-slim` and `rust:*-slim-trixie`
  digests. Get the new digest with
  `docker buildx imagetools inspect debian:trixie-slim`.
- A `Cargo.lock` change usually changes the Shard client plugin hash. Run the
  **Shard client plugins** workflow (`gh workflow run shard-plugins.yml --ref main`).
  Its commit does not start CI, so start it with
  `gh workflow run ci.yml --ref main`.

CI must be green on main before a release: `release.yml` only publishes a tag
whose commit has a successful `ci.yml` run on main.

## 3. Release

From main's head, after CI is green:

```sh
cargo fmt --all
./release.sh <version>            # bumps every manifest and lockfile
git add -u -- .                   # only the release files; check `git status`
git commit -m "chore: release <version>"
git tag -a v<version> -m "Release v<version>"
git push --follow-tags
```

`release.yml` waits for `ci.yml` on the tagged commit, then publishes the
binaries, npm packages and images. Then check that everything published:

```sh
npm view @pylonsync/cli@<v> version
npm view @pylonsync/sdk@<v> version
npm view @pylonsync/react@<v> version
docker buildx imagetools inspect ghcr.io/pylonsync/pylon:<v>
docker buildx imagetools inspect ghcr.io/pylonsync/pylon-builder:<v>
trivy image --severity HIGH,CRITICAL --ignore-unfixed ghcr.io/pylonsync/pylon:<v>
```

## 4. Stack0 Cloud fleet move (the owner approves)

The fleet release (`PYLON_RELEASE` on `pylon-cloud`) sets the image for new
deploys, the builder and dev boxes. In `stack0dev/stack0-apps`:

```sh
gh workflow run pylon-release.yml -f version=<v>
```

It builds the Build workspace image for `<v>` first, then sets
`PYLON_RELEASE`. Check it with
`fly ssh console -a pylon-cloud -C 'printenv PYLON_RELEASE'`.

## 5. Apps that keep an older image

A Cloud app keeps its runtime image across deploys. It moves only when its
`@pylonsync/*` version changes or someone runs `pylon runtime <version>`.
Cloud's `reportRuntimeDrift` cron lists apps behind the fleet.

- **Same minor line as the fix** (for example 0.22.x to 0.22.56): move the
  runtime in place. Cloud swaps one machine at a time and puts the old image
  back if health checks fail. In the app directory: `pylon runtime <v>`, or
  the dashboard's Runtime card.
- **Older minor line** (for example 0.18.x): the binary cannot move without
  rebuilding the app (`lib/runtimeCompat.ts` refuses it). The app's
  `@pylonsync/*` packages must be upgraded and the app deployed again. Tell
  the app's owner, with the deadline from `SECURITY.md`.

## 6. Stack0 images that are not Pylon

| Image | Where | How it ships |
| --- | --- | --- |
| Model proxy | `stack0-apps/apps/model-proxy` | `cd apps/model-proxy && fly deploy --strategy bluegreen` |
| Studio director | `stack0-apps/apps/studio-director` | `fly deploy --build-only --push --image-label director-<yyyymmdd-hhmm> -a stack0-studio-director`, then `STUDIO_DIRECTOR_IMAGE` in `apps/cloud/fly.toml` (the push deploys Cloud) |
| SEO agent | `stack0-apps/apps/seo-agent` | same, with `-a stack0-seo-agent` and `SEO_AGENT_IMAGE` |
| Build workspace | `stack0-apps/apps/build/workspace-image` | the `build-workspace-image` workflow (runs on push and from `pylon-release.yml`) |
| Cloud control plane | `stack0-apps/apps/cloud` | the `deploy-cloud` workflow on push |
| pylonsync.com | `pylon/apps/site` | the `deploy-site` workflow |

Each Dockerfile pins its base image by digest, and Dependabot moves the
digests weekly.

## 7. Verify

Scan what is running, not what was built:

```sh
fly machines list -a <app> --json | jq -r '.[].config.image'
trivy image --severity HIGH,CRITICAL --ignore-unfixed <that image>
```
