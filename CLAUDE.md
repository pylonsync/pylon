# Pylon — agent operating rules

## STOP HALF-ASSING

When the user asks for a feature, ship it end-to-end. The following count as half-assing and are forbidden:

- **Insecure bypass flags.** Don't ship `PYLON_*_INSECURE_NO_VERIFY=1` style escape hatches and call the feature done. Implement the verification. If the verification needs xmlsec1 + libxml2, add them to the Dockerfile.
- **String-scanning instead of a real parser.** Don't `pluck_attr` regexes when the answer is a vetted XML / JWT / ASN.1 library. Use the library, even if the system dep cost feels heavy.
- **"Deferred to follow-up" for the security-critical core.** Storage + endpoints without signature verification ≠ SAML support. Either ship the verifier or don't ship the surface.
- **In-memory-only stores when the feature requires persistence.** Don't ship a config UI whose data evaporates on restart.
- **Marking tasks complete before the work is done.** Including "I'll do that follow-up next" handwaving.
- **Pretending an adversarial review happened.** If the user asked for a code-reviewer-agent pass, run it. Mental walkthrough doesn't count.
- **Claiming "deploy should resolve it" without verifying.** Cloud features get verified in a browser. Binary features get smoke-tested.

The user runs production systems (Pylon Cloud, Stack0). A half-shipped feature that looks complete burns hours when they hit the gap in production. The cost of doing it right the first time is always lower than the cost of being told to redo it.

If a real implementation requires touching the Dockerfile + Cargo.toml + 5 crates + 800 lines of integration: **do all of it**. Don't take the shorter path that produces a non-working result.

## Other rules

See ~/.claude/CLAUDE.md for the global rules (no `rm -rf`, no Co-Authored-By, no fake time estimates, no shortcuts).
