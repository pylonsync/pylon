#!/usr/bin/env python3
"""Daily security report: merge scanner output, diff against yesterday.

Usage:
    security_daily.py <results-dir> <previous-findings.json or ""> <out-dir>

<results-dir> holds whatever the workflow produced (missing files are fine):
    trivy-<key>.json      trivy sbom/image --format json (one per SBOM)
    osv.json              osv-scanner --format json
    ai-review.md          the AI review of the last day's commits
    fix-prs.md            open security fix pull requests, one per line
    meta.json             {"date", "repo", "run_url", "images": [...],
                           "checks": {"cargo audit": "passed", ...},
                           "labels": {"<key>": "image (apps that run it)"}}

Writes to <out-dir>:
    findings.json   every finding, for tomorrow's diff (kept as an artifact)
    report.md       the job summary
    slack.txt       the Slack message (plain text)

Severity: CRITICAL, HIGH, MEDIUM, LOW. Only findings with a fixed version
are counted as actionable; the rest are listed as "no fix yet".
"""

import json
import os
import sys

ORDER = ["CRITICAL", "HIGH", "MEDIUM", "LOW", "UNKNOWN"]


def cvss_band(score):
    if score is None:
        return "UNKNOWN"
    if score >= 9.0:
        return "CRITICAL"
    if score >= 7.0:
        return "HIGH"
    if score >= 4.0:
        return "MEDIUM"
    if score > 0:
        return "LOW"
    return "UNKNOWN"


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def from_trivy(path, labels):
    data = load(path)
    out = []
    if not data:
        return out
    # meta.json may label each scan ("image (apps that run it)"); otherwise
    # the image reference, without the digest.
    stem = os.path.basename(path)[len("trivy-"):-len(".json")]
    target = labels.get(stem) or (data.get("ArtifactName") or stem).split("@sha256:")[0]
    for result in data.get("Results") or []:
        for v in result.get("Vulnerabilities") or []:
            out.append(
                {
                    "id": v["VulnerabilityID"],
                    "severity": v.get("Severity", "UNKNOWN"),
                    "package": v.get("PkgName", "?"),
                    "version": v.get("InstalledVersion", ""),
                    "fixed": v.get("FixedVersion") or "",
                    "where": target,
                    "url": v.get("PrimaryURL") or f"https://osv.dev/{v['VulnerabilityID']}",
                }
            )
    return out


def osv_fixed(vuln, name):
    fixes = []
    for affected in vuln.get("affected", []):
        if affected.get("package", {}).get("name") != name:
            continue
        for r in affected.get("ranges", []):
            fixes += [e["fixed"] for e in r.get("events", []) if "fixed" in e]
    return ", ".join(sorted(set(fixes)))


def from_osv(path):
    data = load(path)
    out = []
    if not data:
        return out
    for result in data.get("results", []):
        source = result.get("source", {}).get("path", "?").split("/", 3)[-1]
        for pkg in result.get("packages", []):
            name = pkg["package"]["name"]
            vulns = {v["id"]: v for v in pkg.get("vulnerabilities", [])}
            for group in pkg.get("groups", []):
                try:
                    score = float(group.get("max_severity") or "nan")
                except ValueError:
                    score = None
                if score != score:  # NaN: no score
                    score = None
                ids = group["ids"]
                out.append(
                    {
                        "id": ids[0],
                        "aliases": ids[1:],
                        "severity": cvss_band(score),
                        "package": name,
                        "version": pkg["package"]["version"],
                        "fixed": ", ".join(
                            sorted({f for i in ids if i in vulns for f in osv_fixed(vulns[i], name).split(", ") if f})
                        ),
                        "where": source,
                        "url": f"https://osv.dev/{ids[0]}",
                    }
                )
    return out


def key(f):
    return f"{f['where']}|{f['package']}|{f['version']}|{f['id']}"


def dedupe(findings):
    seen = {}
    for f in findings:
        k = key(f)
        if k not in seen or ORDER.index(f["severity"]) < ORDER.index(seen[k]["severity"]):
            seen[k] = f
    return sorted(seen.values(), key=lambda f: (ORDER.index(f["severity"]), f["package"], f["id"]))


def line(f):
    fix = f"fixed in {f['fixed']}" if f["fixed"] else "no fix yet"
    return f"{f['severity']} {f['id']} in {f['package']} {f['version']} ({f['where']}), {fix}: {f['url']}"


def main():
    results, previous_path, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]
    os.makedirs(out_dir, exist_ok=True)
    meta = load(os.path.join(results, "meta.json")) or {}
    findings = []
    for name in sorted(os.listdir(results)):
        path = os.path.join(results, name)
        if name.startswith("trivy-") and name.endswith(".json"):
            findings += from_trivy(path, meta.get("labels", {}))
    findings += from_osv(os.path.join(results, "osv.json"))
    findings = [f for f in dedupe(findings) if f["severity"] in ("CRITICAL", "HIGH", "MEDIUM")]

    previous = load(previous_path) if previous_path else None
    known = {key(f) for f in previous} if isinstance(previous, list) else None
    # With no previous run every finding is new, so the first report lists
    # what is open instead of claiming there is nothing.
    new = [f for f in findings if known is None or key(f) not in known]
    fixable = [f for f in findings if f["fixed"] and f["severity"] in ("CRITICAL", "HIGH")]

    with open(os.path.join(out_dir, "findings.json"), "w") as f:
        json.dump(findings, f, indent=1)

    def read(name):
        try:
            with open(os.path.join(results, name)) as f:
                return f.read().strip()
        except OSError:
            return ""

    ai = read("ai-review.md")
    prs = read("fix-prs.md")
    date = meta.get("date", "")
    repo = meta.get("repo", "")
    run = meta.get("run_url", "")

    md = [f"# Security check, {date} ({repo})", ""]
    md.append(f"Scanned: {', '.join(meta.get('images', [])) or 'lockfiles only'}.")
    checks = meta.get("checks", {})
    if checks:
        md.append("Checks: " + ", ".join(f"{k} {v}" for k, v in checks.items()) + ".")
    md.append(f"Findings (medium and up): {len(findings)}. High or critical with a fix: {len(fixable)}.")
    md.append(
        "New since yesterday: no previous run, so everything open is listed as new." if known is None
        else f"New since yesterday: {len(new)}."
    )
    md.append("")
    if new:
        md += ["## New", ""] + [f"- {line(f)}" for f in new] + [""]
    if fixable:
        md += ["## High or critical with a fix", ""] + [f"- {line(f)}" for f in fixable] + [""]
    md += ["## AI review of the last day's commits", "", ai or "Not run.", ""]
    md += ["## Fix pull requests", "", prs or "None open.", ""]
    md += [f"Run: {run}"]
    with open(os.path.join(out_dir, "report.md"), "w") as f:
        f.write("\n".join(md) + "\n")

    # Slack: one message, plain words.
    ai_has_findings = bool(ai) and not ai.lower().startswith(("no findings", "not run", "skipped"))
    if not new and not ai_has_findings and all(v == "passed" for v in checks.values()):
        slack = f"Security check, {date} ({repo}): no new issues. {run}"
    else:
        parts = [f"Security check, {date} ({repo}): {len(new)} new finding(s)."]
        # Slack lists new high and critical findings; mediums are a count
        # (the run's summary lists them all).
        mediums = sum(1 for f in new if f["severity"] == "MEDIUM")
        if mediums:
            parts.append(f"{mediums} new medium finding(s) are in the run's summary.")
        for sev in ("CRITICAL", "HIGH"):
            group = [f for f in new if f["severity"] == sev]
            if group:
                parts.append(f"{sev.title()}:")
                parts += [f"- {line(f)}" for f in group[:15]]
                if len(group) > 15:
                    parts.append(f"- and {len(group) - 15} more")
        if ai_has_findings:
            parts += ["AI review:", ai[:2500]]
        if prs:
            parts += ["Fix pull requests:", prs[:1500]]
        failed = [k for k, v in checks.items() if v != "passed"]
        if failed:
            parts.append("Failed checks: " + ", ".join(failed) + ".")
        if fixable:
            parts.append(f"{len(fixable)} high or critical finding(s) have a fix waiting; see the run.")
        parts.append(run)
        slack = "\n".join(parts)
    with open(os.path.join(out_dir, "slack.txt"), "w") as f:
        f.write(slack + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
