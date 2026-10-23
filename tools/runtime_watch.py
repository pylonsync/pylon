#!/usr/bin/env python3
"""Runtime watch: are our pinned runtimes behind a security release?

Usage:
    runtime_watch.py <out.json> <repo-dir> [<repo-dir> ...]

Trivy does not reliably see runtimes copied into an image (Bun especially),
so this reads the versions we pin and asks the projects themselves:

- Node: `node:<version>` images, `NODE_VERSION=v<version>` and `node-version:`
  in workflows, against https://nodejs.org/dist/index.json, whose releases
  carry a `security` flag. Behind = a newer release in the same major has
  `security: true`.
- Bun: `oven/bun:<version>`, `bun-version:` and `bun@<version>`, against
  Bun's GitHub releases. Behind = a newer release whose notes
  (https://bun.com/blog/bun-v<version>) have a Security section.
- Rust: `RUST_VERSION=` / `rust:<version>` against the stable channel.
  Reported when behind; Rust releases are rarely security releases.
- Base images pinned as `name:tag@sha256:<digest>`: reported when the tag
  now points at a newer build (`docker buildx imagetools inspect`).

Writes {"behind": [...], "info": [...]}; each entry is one plain sentence.
"""

import glob
import json
import os
import re
import subprocess
import sys
import urllib.request


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "pylon-runtime-watch"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "replace")


def vtuple(v):
    return tuple(int(x) for x in re.findall(r"\d+", v)[:3])


def files(repo):
    pats = ["**/Dockerfile", "**/*.Dockerfile", ".github/workflows/*.yml", "package.json", "rust-toolchain*"]
    out = []
    for p in pats:
        for f in glob.glob(os.path.join(repo, p), recursive=True):
            if "/node_modules/" in f or "/target/" in f or "/.deepsec/" in f:
                continue
            out.append(f)
    return sorted(set(out))


def scan(repos):
    pins = {"node": {}, "bun": {}, "rust": {}, "digest": {}}
    for repo in repos:
        for f in files(repo):
            rel = os.path.relpath(f, os.path.dirname(repo.rstrip("/")))
            text = open(f, encoding="utf-8", errors="replace").read()
            for m in re.finditer(r"\bnode:(\d+\.\d+\.\d+)", text):
                pins["node"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"NODE_VERSION=v?(\d+\.\d+\.\d+)", text):
                pins["node"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"node-version:\s*[\"']?(\d+\.\d+\.\d+)", text):
                pins["node"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"oven/bun:(\d+\.\d+\.\d+)", text):
                pins["bun"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"bun-version:\s*[\"']?(\d+\.\d+\.\d+)", text):
                pins["bun"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"bun@(\d+\.\d+\.\d+)", text):
                pins["bun"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"bun-v(\d+\.\d+\.\d+)", text):
                pins["bun"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"RUST_VERSION=(\d+\.\d+(?:\.\d+)?)", text):
                pins["rust"].setdefault(m.group(1), set()).add(rel)
            for m in re.finditer(r"([a-z0-9./_-]+:[A-Za-z0-9._-]+)@(sha256:[0-9a-f]{64})", text):
                pins["digest"].setdefault((m.group(1), m.group(2)), set()).add(rel)
    return pins


def where(paths):
    paths = sorted(paths)
    return ", ".join(paths[:3]) + (f" and {len(paths) - 3} more" if len(paths) > 3 else "")


def check_node(pins, behind, info):
    if not pins:
        return
    index = json.loads(get("https://nodejs.org/dist/index.json"))
    for pinned, paths in pins.items():
        major = pinned.split(".")[0]
        newer = [r for r in index if r["version"].lstrip("v").split(".")[0] == major
                 and vtuple(r["version"]) > vtuple(pinned)]
        security = [r for r in newer if r.get("security")]
        if security:
            latest = max(newer, key=lambda r: vtuple(r["version"]))["version"].lstrip("v")
            first = min(security, key=lambda r: vtuple(r["version"]))
            behind.append(
                f"Node {pinned} ({where(paths)}) is behind security release "
                f"{first['version'].lstrip('v')} ({first['date']}); move to {latest}."
            )
        elif newer:
            info.append(f"Node {pinned} ({where(paths)}): {len(newer)} newer release(s), none flagged security.")


def check_bun(pins, behind, info):
    if not pins:
        return
    releases = json.loads(get("https://api.github.com/repos/oven-sh/bun/releases?per_page=30"))
    versions = [r["tag_name"].removeprefix("bun-v") for r in releases if r["tag_name"].startswith("bun-v")]
    latest = max(versions, key=vtuple)
    notes = {}
    for pinned, paths in pins.items():
        newer = sorted((v for v in versions if vtuple(v) > vtuple(pinned)), key=vtuple)
        flagged = []
        for v in newer[-8:]:
            if v not in notes:
                try:
                    page = get(f"https://bun.com/blog/bun-v{v}")
                    notes[v] = bool(re.search(r">\s*Security\s*<|id=\"security\"|#\s*Security\b", page))
                except Exception:
                    notes[v] = None
            if notes[v]:
                flagged.append(v)
        if flagged:
            behind.append(
                f"Bun {pinned} ({where(paths)}) is behind security release {flagged[0]}; move to {latest}."
            )
        elif newer:
            info.append(f"Bun {pinned} ({where(paths)}): newer releases up to {latest}, none with a Security section.")


def check_rust(pins, behind, info):
    if not pins:
        return
    toml = get("https://static.rust-lang.org/dist/channel-rust-stable.toml")
    m = re.search(r'\[pkg\.rust\]\s*\nversion = "(\d+\.\d+\.\d+)', toml)
    if not m:
        return
    stable = m.group(1)
    for pinned, paths in pins.items():
        if vtuple(pinned)[:2] < vtuple(stable)[:2]:
            info.append(f"Rust {pinned} ({where(paths)}) is behind stable {stable}.")


def check_digests(pins, behind, info):
    for (ref, digest), paths in pins.items():
        try:
            out = subprocess.run(
                ["docker", "buildx", "imagetools", "inspect", ref],
                capture_output=True, text=True, timeout=60,
            ).stdout
        except Exception:
            continue
        m = re.search(r"Digest:\s+(sha256:[0-9a-f]{64})", out)
        if m and m.group(1) != digest:
            info.append(
                f"{ref} ({where(paths)}) has a newer build than the pinned digest; "
                "Dependabot's weekly base-image PR moves it, or move it now if Trivy flags the old one."
            )


def main():
    out, repos = sys.argv[1], sys.argv[2:]
    pins = scan(repos)
    behind, info = [], []
    for check, key in [(check_node, "node"), (check_bun, "bun"), (check_rust, "rust"), (check_digests, "digest")]:
        try:
            check(pins[key], behind, info)
        except Exception as e:  # one source being down must not hide the others
            info.append(f"Could not check {key}: {e}")
    with open(out, "w") as f:
        json.dump({"behind": behind, "info": info}, f, indent=1)
    for line in behind:
        print("BEHIND:", line)
    for line in info:
        print("info:", line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
