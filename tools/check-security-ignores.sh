#!/usr/bin/env bash
# Fails when an ignored advisory is past its review date. cargo audit and
# cargo deny have no expiry of their own, so each ignore in .cargo/audit.toml
# and deny.toml says "Review by YYYY-MM-DD"; this checks those dates.
# osv-scanner.toml (ignoreUntil) and .trivyignore.yaml (expired_at) expire on
# their own. SECURITY.md has the policy.
set -euo pipefail
cd "$(dirname "$0")/.."

today="$(date -u +%Y-%m-%d)"
status=0
for f in .cargo/audit.toml deny.toml; do
	while IFS= read -r line; do
		date="$(printf '%s\n' "$line" | sed -E 's/.*Review by ([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/')"
		if [[ "$date" < "$today" ]]; then
			echo "::error file=$f::An ignored advisory was due for review on $date: $line"
			status=1
		fi
	done < <(grep -E 'Review by [0-9]{4}-[0-9]{2}-[0-9]{2}' "$f" || true)
	if ! grep -qE 'Review by [0-9]{4}-[0-9]{2}-[0-9]{2}' "$f" && grep -qE 'RUSTSEC-[0-9]{4}-[0-9]{4}' "$f"; then
		echo "::error file=$f::Every ignored advisory needs a 'Review by YYYY-MM-DD' date."
		status=1
	fi
done
exit "$status"
