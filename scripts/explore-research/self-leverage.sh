#!/usr/bin/env bash
# scripts/explore-research/self-leverage.sh — discovery researcher (self-leverage).
#
# Scans every point in the trio prose + scripts where a human decision /
# intervention / relaunch is still required, and checks each against the
# autonomy-scope rule:
#   - low-stakes decisions should auto-resolve;
#   - only run/defer/refine + destructive-remote actions reach the operator.
#
# Cap: 50 candidates per round.
# Default weight: 0.6.
#
# Output: JSON to stdout matching schemas/autospec-explore-proposal.schema.json
# (extended contract with severity + named_consumer).

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
MAX_PROPOSALS=50

cd "$REPO_ROOT" || { echo '{"source":"self-leverage","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"self-leverage","proposals":[]}'
    exit 0
fi

# Collect intervention-signal lines from skill prose and scripts.
prose_tmp="$(mktemp -t sl-prose.XXXXXX)"
script_tmp="$(mktemp -t sl-scripts.XXXXXX)"
trap 'rm -f "$prose_tmp" "$script_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Skill prose: SKILL.md files and docs/specs/*.md
    git grep -n -I -E \
        '(manually|operator must|human must|ask the operator|AskUserQuestion|requires human|intervention|relaunch|re-launch|you must|please run|operator should confirm|operator confirm)' \
        -- 'skills/**/*.md' 'skills/*.md' 'docs/specs/*.md' 'AGENTS.md' \
        2>/dev/null | head -n 200 > "$prose_tmp" || true

    # Scripts: explicit prompts / read / confirm patterns
    git grep -n -I -E \
        '(read -r|read -p|AskUserQuestion|printf.*\?|echo.*\?.*read|confirm|pause|press enter|--interactive)' \
        -- 'scripts/*.sh' 'scripts/**/*.sh' \
        2>/dev/null | head -n 200 > "$script_tmp" || true
fi

export AUTOSPEC_MAX_PROPOSALS="$MAX_PROPOSALS"

python3 - "$prose_tmp" "$script_tmp" <<'PY'
import json, os, re, sys

prose_path  = sys.argv[1]
script_path = sys.argv[2]
cap         = int(os.environ.get("AUTOSPEC_MAX_PROPOSALS", "50"))

proposals = []

def add(title, evidence, complexity="small", confidence=0.65,
        severity="operability", named_consumer=""):
    if len(proposals) >= cap:
        return
    proposals.append({
        "title": title,
        "evidence": evidence,
        "estimated_complexity": complexity,
        "confidence": confidence,
        "severity": severity,
        "named_consumer": named_consumer,
    })

# Patterns that indicate a human-in-the-loop requirement.
# Split into: (a) legitimately requiring operator (run/defer/refine/destructive)
# and (b) low-stakes that should auto-resolve.
OPERATOR_LEGIT = re.compile(
    r'(run\b|defer\b|refine\b|destructive|push.*force|delete.*branch|'
    r'merge.*main|reset.*hard|--admin|irreversible)',
    re.IGNORECASE,
)
HUMAN_SIGNAL = re.compile(
    r'(manually|operator must|human must|ask the operator|AskUserQuestion|'
    r'requires human|intervention|relaunch|re-launch|you must|please run|'
    r'operator should confirm|operator confirm|read -[rp]|confirm\b|pause\b|'
    r'press enter)',
    re.IGNORECASE,
)

seen = set()

def process_file(path, source_label):
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            lines = fh.readlines()
    except FileNotFoundError:
        return

    for line in lines:
        line = line.rstrip("\n")
        if not line.strip():
            continue
        # Format: file:lineno:content
        m = re.match(r'^(.+?):(\d+):(.+)$', line)
        if not m:
            continue
        fpath, lineno, content = m.group(1), m.group(2), m.group(3).strip()

        if not HUMAN_SIGNAL.search(content):
            continue

        # Skip if this looks like a legitimately operator-facing action.
        if OPERATOR_LEGIT.search(content):
            continue

        key = f"{fpath}:{lineno}"
        if key in seen:
            continue
        seen.add(key)

        snippet = content[:120].replace('"', "'")
        add(
            f"feat(autonomy): auto-resolve human-in-loop step at {fpath}:{lineno}",
            f"{source_label} hit at {fpath}:{lineno}: \"{snippet}\" — this step requires manual operator action but appears low-stakes per the autonomy-scope rule (should auto-resolve; only run/defer/refine/destructive reach the operator).",
            complexity="medium",
            confidence=0.65,
            severity="operability",
            named_consumer="/autospec-explore autonomy-scope rule; /autospec-run --autonomous",
        )

process_file(prose_path,  "prose intervention signal")
process_file(script_path, "script interaction pattern")

print(json.dumps({"source": "self-leverage", "proposals": proposals}))
PY
