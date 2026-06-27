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

# Collect interactive call sites from EXECUTABLE SCRIPTS ONLY. Markdown prose is
# deliberately NOT scanned: matching prose that *describes* a human-in-loop step
# (including deliberate safety gates) and proposing to "auto-resolve" it was this
# researcher's dominant false-positive mode. Only a real interactive call site in
# a script is an actionable, machine-confirmable self-leverage point.
script_tmp="$(mktemp -t sl-scripts.XXXXXX)"
trap 'rm -f "$script_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Scripts: genuine interactive PROMPT statements only — a real `read -p`/`-rp`
    # (the -p flag carries a prompt string). Deliberately NARROW: bare `read -r`
    # (pipe/while-loop input) and bare token mentions of AskUserQuestion /
    # --interactive are excluded because they appear overwhelmingly in comments,
    # --help text, and prose rather than as call sites (the dominant false
    # positive). Comment lines are dropped in Python below.
    git grep -n -I -E \
        'read -[a-z]*p[[:space:]]' \
        -- 'scripts/*.sh' 'scripts/**/*.sh' \
        2>/dev/null | head -n 200 > "$script_tmp" || true
fi

export AUTOSPEC_MAX_PROPOSALS="$MAX_PROPOSALS"

python3 - "$script_tmp" <<'PY'
import json, os, re, sys

script_path = sys.argv[1]
cap         = int(os.environ.get("AUTOSPEC_MAX_PROPOSALS", "50"))

proposals = []

def add(title, evidence, gap_check, complexity="medium", confidence=0.65,
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
        "gap_check": gap_check,
    })

# An interactive call site that legitimately requires the operator
# (run/defer/refine/destructive actions) is NOT a self-leverage candidate.
OPERATOR_LEGIT = re.compile(
    r'(run\b|defer\b|refine\b|destructive|push.*force|delete.*branch|'
    r'merge.*main|reset.*hard|--admin|irreversible)',
    re.IGNORECASE,
)
# Genuine interactive PROMPT statement: `read -p`/`-rp` (the -p flag carries a
# prompt). Narrow by design — see the grep comment above.
CALL_SITE = re.compile(r'read -[a-z]*p\s')
# Lines that are themselves grep/regex PATTERN definitions match the token as
# data, not as a call site (a researcher/aggregator matching its own source).
# Skip them — this is the self-match false-positive class.
NOISE_CONTEXT = re.compile(
    r'(git grep|grep -|ls-files|re\.(compile|search|match|sub)|'
    r'CALL_SITE|HUMAN_SIGNAL|NOISE_CONTEXT|pathspec|head -n 200)',
    re.IGNORECASE,
)

seen = set()

def process_scripts(path):
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            lines = fh.readlines()
    except FileNotFoundError:
        return

    for line in lines:
        line = line.rstrip("\n")
        if not line.strip():
            continue
        # git grep format: file:lineno:content
        m = re.match(r'^(.+?):(\d+):(.+)$', line)
        if not m:
            continue
        fpath, lineno, content = m.group(1), m.group(2), m.group(3).strip()

        if content.startswith("#"):
            continue          # comment / help-text line, not a call site
        if not CALL_SITE.search(content):
            continue
        if OPERATOR_LEGIT.search(content):
            continue
        if NOISE_CONTEXT.search(content):
            continue          # a grep/regex pattern line, not a real call site

        key = f"{fpath}:{lineno}"
        if key in seen:
            continue
        seen.add(key)

        # The gap_check needle is a fixed-string slice of the matched line that
        # the aggregator re-confirms is STILL a present call site in fpath
        # (kind=present). A stripped slice is always a substring of the file.
        needle = content[:80]
        snippet = needle.replace('"', "'")
        add(
            f"feat(autonomy): auto-resolve interactive call site at {fpath}:{lineno}",
            f"interactive call site at {fpath}:{lineno}: \"{snippet}\" — a real "
            f"prompt/read in shell code that appears low-stakes per the "
            f"autonomy-scope rule (should auto-resolve; only "
            f"run/defer/refine/destructive reach the operator).",
            gap_check={"kind": "present", "needle": needle, "haystack": fpath},
            complexity="medium",
            confidence=0.65,
            severity="operability",
            named_consumer="/autospec-explore autonomy-scope rule; /autospec-run --autonomous",
        )

process_scripts(script_path)

print(json.dumps({"source": "self-leverage", "proposals": proposals}))
PY
