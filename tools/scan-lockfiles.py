#!/usr/bin/env python3
"""Scan supported lockfiles tracked by Git. Write OSV JSON to stdout."""

import argparse
from pathlib import Path
import subprocess
import sys


SUPPORTED_NAMES = {
    "Cargo.lock", "bun.lock", "bun.lockb", "Package.resolved",
    "package-lock.json", "npm-shrinkwrap.json", "yarn.lock", "pnpm-lock.yaml",
    "composer.lock", "Gemfile.lock", "poetry.lock", "Pipfile.lock", "uv.lock",
    "go.sum", "packages.lock.json", "pubspec.lock", "mix.lock",
}


def tracked_lockfiles(paths):
    return sorted(path for path in paths if Path(path).name in SUPPORTED_NAMES)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scanner", default="osv-scanner")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    lockfiles = tracked_lockfiles(tracked)
    if not lockfiles:
        print("No supported tracked lockfiles found", file=sys.stderr)
        return 2
    command = [args.scanner, "scan", "source", "--config", "osv-scanner.toml", "--format", "json"]
    for path in lockfiles:
        print(f"Scanning {path}", file=sys.stderr)
        command.extend(["--lockfile", path])
    return subprocess.run(command, cwd=root, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())
