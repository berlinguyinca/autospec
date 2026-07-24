#!/usr/bin/env bash
# scripts/explore-research/codebase-signals.sh — deterministic researcher #3 of 4.
#
# Greps the repo for TODO/FIXME comments, optionally parses a coverage report
# at .autospec/coverage.json or coverage/*.json, and emits one proposal per
# signal.
#
# Cap: 200 signals per round.
#
# Output: JSON to stdout per the spec contract.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
MAX_SIGNALS=200

cd "$REPO_ROOT" || { echo '{"source":"codebase-signals","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"codebase-signals","proposals":[]}'
    exit 0
fi

todo_tmp="$(mktemp -t codebase-signals.XXXXXX)"
cov_tmp="$(mktemp -t codebase-signals-cov.XXXXXX)"
trap 'rm -f "$todo_tmp" "$cov_tmp"' EXIT

# Collect TODO/FIXME signals from tracked files (capped).
#
# Precision: require a real annotation shape (MARKER: or MARKER(owner)) — NOT a
# bare trailing space — so prose like "build me a TODO list CLI" or "FIXME found"
# does not match. Exclude docs/marketing + non-source assets, and this researcher's
# own dir, tests, and lint tooling, which only *document* or exercise the
# markers and would otherwise flag themselves (self-referential false positives).
if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git grep -n -I -E '(TODO|FIXME|XXX|HACK)[:(]' \
        -- ':!*.md' ':!*.html' ':!*.cast' ':!*.svg' ':!docs/' \
           ':!tests/' ':!scripts/explore-research/' ':!scripts/lint-*.sh' 2>/dev/null \
        | head -n "$MAX_SIGNALS" > "$todo_tmp" || true
fi

# Coverage report (optional).
for cov in ".autospec/coverage.json" coverage/coverage-summary.json coverage/*.json; do
    if [ -f "$cov" ]; then
        cp "$cov" "$cov_tmp"
        break
    fi
done

python3 - "$todo_tmp" "$cov_tmp" "$MAX_SIGNALS" <<'PY'
import json, sys, os, re

todo_path = sys.argv[1]
cov_path  = sys.argv[2]
cap       = int(sys.argv[3])

proposals = []

# TODO/FIXME signals.
try:
    with open(todo_path, 'r', encoding='utf-8', errors='ignore') as fh:
        for line in fh:
            line = line.rstrip('\n')
            if not line:
                continue
            # Format: path:lineno:content
            m = re.match(r'^(.+?):(\d+):(.+)$', line)
            if not m:
                continue
            path, lineno, content = m.group(1), m.group(2), m.group(3).strip()
            kind = "TODO"
            for k in ("FIXME","XXX","HACK","TODO"):
                if k in content:
                    kind = k
                    break
            snippet = content[:80].replace('"', "'")
            proposals.append({
                "title": f"chore: address {kind} in {path}:{lineno}",
                "evidence": f"{kind} at {path}:{lineno}: {content[:200]}",
                "estimated_complexity": "small",
                "confidence": 0.55,
            })
            if len(proposals) >= cap:
                break
except FileNotFoundError:
    pass

# Coverage signals (low coverage files).
if len(proposals) < cap and os.path.exists(cov_path) and os.path.getsize(cov_path) > 0:
    try:
        with open(cov_path, 'r', encoding='utf-8', errors='ignore') as fh:
            cov = json.load(fh)
        # istanbul-style: { "/abs/path/file.js": { "lines": {"pct": 42.0}, ... }, ... }
        if isinstance(cov, dict):
            for path, data in cov.items():
                if not isinstance(data, dict):
                    continue
                pct = None
                lines = data.get("lines")
                if isinstance(lines, dict):
                    pct = lines.get("pct")
                elif isinstance(data.get("pct"), (int, float)):
                    pct = data["pct"]
                if pct is None:
                    continue
                try:
                    pct = float(pct)
                except (TypeError, ValueError):
                    continue
                if pct >= 60.0:
                    continue
                proposals.append({
                    "title": f"test: increase coverage for {os.path.basename(path)}",
                    "evidence": f"Coverage report shows {path} at {pct:.1f}% line coverage",
                    "estimated_complexity": "medium",
                    "confidence": 0.60,
                })
                if len(proposals) >= cap:
                    break
    except Exception:
        pass

print(json.dumps({"source": "codebase-signals", "proposals": proposals}))
PY
