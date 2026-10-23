#!/usr/bin/env python3
"""Security review of a diff (or of named source areas) with Claude.

Usage:
    security_ai_review.py daily <diff-file> <out.md>
    security_ai_review.py weekly <area-name> <out.md> <path> [<path> ...]

daily: reviews the last day's commits to main. weekly: a deeper pass over
whole source areas, one area per week (the workflow rotates them).

Reads ANTHROPIC_API_KEY. Writes Markdown: findings with file:line and the
attack, or "No findings." A diff or area too large for one request is split
at file boundaries and reviewed in parts; nothing is cut off.
"""

import os
import sys

import anthropic

MODEL = "claude-opus-5"
# About 300k tokens of input per request, well inside the context window.
MAX_CHARS_PER_REQUEST = 900_000

FOCUS = """You are reviewing changes to Pylon, an open-source app framework
(Rust server, TypeScript SDK and function runtime) that runs many customer
apps on the public internet, and Stack0's apps built on it.

Look for exploitable security bugs only:
- authentication and sessions (sign-in flows, OAuth/OIDC/SAML, passkeys,
  magic codes, tokens, session revocation),
- policies and tenant isolation (rows one user or org can read or write
  that belong to another; admin and internal-function bypasses),
- CORS, CSRF, origin and host checks, open redirects,
- file uploads and downloads (stored XSS, path traversal, signed URLs),
- server-side rendering (XSS in HTML or hydration data, shared caches that
  mix users' data),
- sync and realtime leaks (WebSocket, SSE, change feeds),
- the WebAssembly and function sandbox,
- SSRF from server-side fetches,
- secrets in logs, responses or client bundles,
- denial of service from small inputs (unbounded recursion, allocation,
  missing limits).

For each finding give: severity (critical, high, medium, low), file:line,
the attack as concrete steps, and the fix. Report only what you can tie to
the code shown; no style notes, no speculative "consider adding". If there
is nothing exploitable, answer exactly: No findings."""


def chunks(text: str, sep: str) -> list[str]:
    parts = text.split(sep)
    out, cur = [], ""
    for i, part in enumerate(parts):
        piece = (sep if i else "") + part
        if cur and len(cur) + len(piece) > MAX_CHARS_PER_REQUEST:
            out.append(cur)
            cur = ""
        cur += piece
    if cur:
        out.append(cur)
    return out


def review(client: anthropic.Anthropic, effort: str, intro: str, body: str) -> str:
    message = client.messages.create(
        model=MODEL,
        max_tokens=16000,
        thinking={"type": "adaptive"},
        output_config={"effort": effort},
        system=FOCUS,
        messages=[{"role": "user", "content": f"{intro}\n\n{body}"}],
    )
    if message.stop_reason == "refusal":
        return "The model declined to review this part."
    text = "\n".join(b.text for b in message.content if b.type == "text").strip()
    if message.stop_reason == "max_tokens":
        text += "\n\n(The answer hit the output limit; rerun this part.)"
    return text or "No findings."


def main() -> int:
    mode = sys.argv[1]
    client = anthropic.Anthropic()
    results = []
    if mode == "daily":
        diff_path, out = sys.argv[2], sys.argv[3]
        with open(diff_path, encoding="utf-8", errors="replace") as f:
            diff = f.read()
        parts = chunks(diff, "\ndiff --git ")
        for n, part in enumerate(parts, 1):
            intro = f"Review this diff of the last day's commits to main (part {n} of {len(parts)})."
            results.append(review(client, "high", intro, part))
    elif mode == "weekly":
        area, out, paths = sys.argv[2], sys.argv[3], sys.argv[4:]
        files = []
        for root in paths:
            for dirpath, _, names in os.walk(root):
                for name in sorted(names):
                    if name.endswith((".rs", ".ts", ".tsx")) and "/node_modules/" not in dirpath:
                        p = os.path.join(dirpath, name)
                        with open(p, encoding="utf-8", errors="replace") as f:
                            files.append(f"\n=== FILE {p} ===\n{f.read()}")
        parts = chunks("".join(files), "\n=== FILE ")
        for n, part in enumerate(parts, 1):
            intro = f"Red-team the {area} code below (part {n} of {len(parts)}). Line numbers start at 1 in each file."
            results.append(review(client, "max", intro, part))
    else:
        print(f"unknown mode {mode}", file=sys.stderr)
        return 2
    real = [r for r in results if r.strip() != "No findings."]
    with open(out, "w") as f:
        f.write("\n\n".join(real) if real else "No findings.")
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
