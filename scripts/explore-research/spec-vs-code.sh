#!/usr/bin/env bash
# scripts/explore-research/spec-vs-code.sh — deterministic researcher #1 of 4.
#
# Walks docs/specs/**.md, extracts acceptance criteria (`- [ ]` checkboxes),
# greps the repo for matching code, and emits one proposal per unmatched AC.
#
# Cap: 100 specs scanned per round.
#
# Output: JSON to stdout matching the contract in
# docs/specs/2026-05-29-autospec-explore-design.md — Research cycle contract.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
SPEC_GLOB="${AUTOSPEC_SPEC_GLOB:-docs/specs}"
MAX_SPECS=100

cd "$REPO_ROOT" || { echo '{"source":"spec-vs-code","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"spec-vs-code","proposals":[]}'
    exit 0
fi

# Collect specs (capped).
specs=()
if [ -d "$SPEC_GLOB" ]; then
    while IFS= read -r f; do
        specs+=("$f")
        [ "${#specs[@]}" -ge "$MAX_SPECS" ] && break
    done < <(find "$SPEC_GLOB" -type f -name '*.md' 2>/dev/null | sort)
fi

python3 - "${specs[@]}" <<'PY'
import json, os, re, sys, subprocess

specs = sys.argv[1:]
proposals = []
ac_pat = re.compile(r'^\s*-\s*\[\s*\]\s+(.+)$')

def has_code_signal(text, spec_path):
    # Extract identifier-like tokens (>=4 chars, alnum/underscore/dash)
    tokens = re.findall(r'[A-Za-z][A-Za-z0-9_\-]{3,}', text)
    # Skip pure stopwords
    stop = {'the','and','for','with','from','that','this','must','should',
            'when','then','will','have','add','use','via','into','onto',
            'each','also','only','also','tests','test','code','file','files',
            'spec','specs','main','docs'}
    tokens = [t for t in tokens if t.lower() not in stop]
    if not tokens:
        return True  # nothing to grep; assume covered
    # Try first 3 distinctive tokens
    for tok in tokens[:3]:
        try:
            r = subprocess.run(
                ['git','grep','-l','-F','-i','--',tok],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                timeout=5,
            )
            hits = [l for l in r.stdout.decode('utf-8','ignore').splitlines()
                    if l and l != spec_path]
            if hits:
                return True
        except Exception:
            pass
    return False

for spec in specs:
    try:
        with open(spec, 'r', encoding='utf-8', errors='ignore') as fh:
            lines = fh.readlines()
    except Exception:
        continue
    for i, line in enumerate(lines, start=1):
        m = ac_pat.match(line)
        if not m:
            continue
        ac_text = m.group(1).strip()
        if len(ac_text) < 8:
            continue
        if has_code_signal(ac_text, spec):
            continue
        title = f"feat: implement unmatched AC from {spec}"
        # truncate ac_text to keep title bounded
        snippet = ac_text[:80].replace('`','').replace('"',"'")
        proposals.append({
            "title": f"feat: implement \"{snippet}\" from {spec}",
            "evidence": f"Acceptance criterion at {spec}:{i} has no matching code: {ac_text[:200]}",
            "estimated_complexity": "medium",
            "confidence": 0.75,
        })

print(json.dumps({"source": "spec-vs-code", "proposals": proposals}))
PY
