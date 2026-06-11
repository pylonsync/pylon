# Overnight hardening loop

Paste this into `/loop` to run it. It is checklist-driven and resumable: state
lives in `tools/overnight-scoreboard.md` (create it on first run from the
template at the bottom). Each turn: pick the highest-priority RED item, drive it
to VERIFIED-GREEN, mark it, commit, move on. Stop when every item is green or the
budget is spent. Write a `## Morning report` to the scoreboard before ending.

## Mission

Get Pylon launch-ready: **(1) super robust, (2) DX spot-on, (3) every example
working 100%.** Quality over coverage — a smaller set of genuinely-verified
greens beats a big list of "looks done."

## Hard guardrails (never violate)

- **Commit to `main` only when CI would stay green.** Run the relevant tests +
  `cargo check` + `tsc`/bundle before each commit. If you can't verify it, don't
  commit it.
- **NOTHING outward-facing.** No `release.sh --tag`, no `git tag`, no npm
  publish, no ghcr push, no `pylon deploy`, no `flyctl`, no cloud machine
  changes. Code, tests, docs, examples only.
- **Do not touch `examples/world3d/` uncommitted files** or anything a concurrent
  instance is editing — check `git status` first; leave others' dirty files
  alone.
- **No half-assing** (per CLAUDE.md): real implementations, real tests, real
  adversarial review. A finding isn't "reviewed" because you thought about it —
  spawn skeptic agents. An example isn't "working" until it rendered + its core
  mutation succeeded **in a browser** with zero console/server errors.
- Every fix ships **with a non-vacuous regression test** that fails before / passes
  after.
- One logical change per commit. Conventional commit messages.

## Phase order (examples first, then robustness, then DX)

### Phase 1 — Examples 100%
Fan out (Workflow) one agent per `examples/*` to read its code + flag obvious
breakage. Then, for each example, **you** (main loop, with browser tools):
delete its dev DB → `pylon dev` on a fresh port → hit every route (curl + the
chrome tools) → exercise the core create/mutation → assert: boots, SSR renders
real data, the realtime/optimistic path works, **zero** errors in the server log
and browser console. Fix what's broken in the example or the framework. Mark the
example green only after a browser screenshot confirms the core flow.
Also: confirm `npm create @pylonsync/pylon` templates scaffold a **standalone**
app (no `workspace:*`) that boots.

### Phase 2 — CI green
`main` CI is RED (stale `integration.rs` tests from the default-deny change, +
possibly others). Make CI green **without weakening coverage**: fix the code or
correctly update the test's expectations (default-deny is correct — see the
memory note; don't "fix" it by re-allowing anon CRUD). Enumerate every failing
job (`gh run view --log-failed`), drive each to pass.

### Phase 3 — Robustness / adversarial
Run the multi-agent adversarial audit pattern (dimensions → find → independent
skeptics verify, majority-refute kills a finding) against the surfaces touched
recently: `field.owner()`/OwnerStampPlugin, example email/password auth, the
Watch/private-data policies, slugs, SSR cache, sync push/pull. Also a
**process-lifecycle sweep**: the orphaned-bun bug is a class — audit every
spawned child + every `Drop`/shutdown path for "survives parent / signal-kill
skips cleanup." Sync-engine parity: diff TS ↔ Rust ↔ Swift behavior, port gaps
both ways. Each confirmed finding → fix + regression test.

### Phase 4 — DX & docs
Audit `pylon dev` startup output, error messages (are they actionable?), the dev
error overlay, hot-reload. Fill docs gaps for every public primitive (check each
`field.*` modifier, each `ctx.*`, each `db.*` hook has a doc entry). Tighten
`pylon init` output.

## Loop mechanics
- Default to `Workflow` for anything parallel (auditing N examples, N adversarial
  dimensions). Use the main loop for browser verification + integration.
- Keep `tools/overnight-scoreboard.md` updated every turn — it's the source of
  truth + the morning report.
- If blocked on something that needs a human (a real product decision, an
  outward-facing action), note it under `## Needs human` and move to the next
  item — never stall the whole loop.

---

### Scoreboard template (create `tools/overnight-scoreboard.md` on first run)

```
# Overnight scoreboard — <date>

## Phase 1 — Examples (RED/GREEN + note)
- [ ] todo-app
- [ ] chat
- [ ] market
- [ ] world3d        (skip if the other instance is actively editing)
- [ ] <…one row per examples/* …>
- [ ] create-pylon templates standalone-deployable

## Phase 2 — CI green
- [ ] ci.yml all jobs pass on main

## Phase 3 — Robustness
- [ ] field.owner adversarial pass
- [ ] auth/Watch policy adversarial pass
- [ ] process-lifecycle sweep (spawned children + Drop/shutdown)
- [ ] sync parity TS/Rust/Swift

## Phase 4 — DX & docs
- [ ] startup/error-message audit
- [ ] docs coverage: every public field/ctx/db primitive

## Needs human
-

## Morning report
-
```
