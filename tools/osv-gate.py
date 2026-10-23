#!/usr/bin/env python3
"""Fail on a high or critical advisory that has a fixed version.

Reads `osv-scanner --format json` output on stdin. Prints every finding,
and exits 1 when one has a CVSS score of 7.0 or more and a fixed version
exists. Lower severities and advisories with no fix are printed only.
Ignores come from osv-scanner.toml, which osv-scanner applies itself.
"""

import json
import sys


def fixed_versions(vuln, name):
    out = []
    for affected in vuln.get("affected", []):
        if affected.get("package", {}).get("name") != name:
            continue
        for r in affected.get("ranges", []):
            out += [e["fixed"] for e in r.get("events", []) if "fixed" in e]
    return out


def main():
    data = json.load(sys.stdin)
    failed = False
    for result in data.get("results", []):
        source = result.get("source", {}).get("path", "?")
        for pkg in result.get("packages", []):
            name = pkg["package"]["name"]
            version = pkg["package"]["version"]
            vulns = {v["id"]: v for v in pkg.get("vulnerabilities", [])}
            for group in pkg.get("groups", []):
                try:
                    score = float(group.get("max_severity") or 0)
                except ValueError:
                    score = 0.0
                fixes = sorted({f for i in group["ids"] if i in vulns for f in fixed_versions(vulns[i], name)})
                blocking = score >= 7.0 and bool(fixes)
                failed |= blocking
                level = "error" if blocking else "notice"
                print(
                    f"::{level} file={source}::{name} {version}: {', '.join(group['ids'])} "
                    f"(CVSS {score or 'n/a'}; fixed in {', '.join(fixes) or 'no release'})"
                )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
