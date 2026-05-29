#!/usr/bin/env bash
# scripts/explore-research/source-analysis.sh — LLM-heavy researcher #5 of 6.
#
# Input: README + AGENTS.md + `tree -L 3` + last-7d commits → LLM call.
# Output: JSON {"source":"source-analysis","proposals":[…]} on stdout.
#
# Graceful degradation: any LLM error / unparseable output / missing harness
# → 0 proposals + a warn to stderr (loop continues per spec).
#
# Test seam: AUTOSPEC_LLM_STUB_OUTPUT — if set, treated as the LLM's raw
# stdout (skips the actual harness dispatch). Same channel used by
# tests/explore/test_explore_researchers_llm.bats.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"

_emit_empty() {
    echo '{"source":"source-analysis","proposals":[]}'
    exit 0
}

cd "$REPO_ROOT" 2>/dev/null || _emit_empty

# ── Assemble context ──────────────────────────────────────────────
README_TXT=""
AGENTS_TXT=""
TREE_TXT=""
COMMITS_TXT=""

[ -f README.md ] && README_TXT="$(head -c 8000 README.md 2>/dev/null || true)"
[ -f AGENTS.md ] && AGENTS_TXT="$(head -c 8000 AGENTS.md 2>/dev/null || true)"

if command -v tree >/dev/null 2>&1; then
    TREE_TXT="$(tree -L 3 2>/dev/null | head -c 4000 || true)"
elif command -v find >/dev/null 2>&1; then
    TREE_TXT="$(find . -maxdepth 3 -not -path '*/.git*' 2>/dev/null | head -c 4000 || true)"
fi

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    COMMITS_TXT="$(git log --since='7 days ago' --pretty=format:'%h %s' 2>/dev/null | head -n 50 || true)"
fi

INSTRUCTION='You are the autospec-explore "source-analysis" researcher. Inspect the
repository README, AGENTS.md, top-level structure, and recent commits, and
propose 1-5 concrete next features or improvements grounded in what you see.
For each, emit a JSON object with fields: title (string, conventional-commit
form), evidence (short string referencing a real file/area), estimated_complexity
(one of "small","medium","large"), confidence (float 0.0-1.0, default 0.5).
Return ONLY a JSON array of these objects — no prose, no markdown fences.'

FULL_INPUT="$INSTRUCTION"$'\n\n## README\n'"$README_TXT"$'\n\n## AGENTS\n'"$AGENTS_TXT"$'\n\n## Tree\n'"$TREE_TXT"$'\n\n## Recent commits\n'"$COMMITS_TXT"

# ── Dispatch LLM ──────────────────────────────────────────────────
LLM_OUT=""
if [ -n "${AUTOSPEC_LLM_STUB_OUTPUT:-}" ]; then
    LLM_OUT="$AUTOSPEC_LLM_STUB_OUTPUT"
else
    _HARNESS_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/lib/autospec-harness-detect.sh"
    if [ -f "$_HARNESS_LIB" ]; then
        # shellcheck source=../lib/autospec-harness-detect.sh
        . "$_HARNESS_LIB"
    fi
    KIND=""
    if declare -F autospec_harness_detect >/dev/null 2>&1; then
        KIND="$(autospec_harness_detect 2>/dev/null || echo claude)"
    fi
    DISPATCHER=""
    case "$KIND" in
        codex) command -v codex >/dev/null 2>&1 && DISPATCHER="$(command -v codex)" ;;
        *)     command -v claude >/dev/null 2>&1 && DISPATCHER="$(command -v claude)" ;;
    esac
    if [ -z "$DISPATCHER" ]; then
        echo "source-analysis: no LLM harness on PATH; graceful degrade" >&2
        _emit_empty
    fi
    if declare -F autospec_harness_dispatcher_safe >/dev/null 2>&1; then
        if ! autospec_harness_dispatcher_safe "$DISPATCHER"; then
            echo "source-analysis: unsafe dispatcher path; graceful degrade" >&2
            _emit_empty
        fi
    fi
    RC=0
    case "$KIND" in
        codex) LLM_OUT="$("$DISPATCHER" exec --reasoning-effort medium "$FULL_INPUT" 2>/dev/null)" || RC=$? ;;
        *)     LLM_OUT="$("$DISPATCHER" -p "$FULL_INPUT" 2>/dev/null)" || RC=$? ;;
    esac
    if [ "$RC" -ne 0 ] || [ -z "$LLM_OUT" ]; then
        echo "source-analysis: LLM error rc=$RC; graceful degrade" >&2
        _emit_empty
    fi
fi

# ── Parse LLM JSON ────────────────────────────────────────────────
if ! command -v python3 >/dev/null 2>&1; then
    _emit_empty
fi

export _LLM_RAW="$LLM_OUT"
python3 - <<'PY'
import json, sys, re, os
raw = os.environ.get("_LLM_RAW","")
out = {"source": "source-analysis", "proposals": []}
m = re.search(r'\[.*\]', raw, re.DOTALL)
arr = None
if m:
    try:
        arr = json.loads(m.group(0))
    except Exception:
        arr = None
if isinstance(arr, list):
    for item in arr:
        if not isinstance(item, dict):
            continue
        title = str(item.get("title","")).strip()
        evidence = str(item.get("evidence","")).strip()
        complexity = item.get("estimated_complexity","medium")
        if complexity not in ("small","medium","large"):
            complexity = "medium"
        try:
            conf = float(item.get("confidence", 0.5))
        except Exception:
            conf = 0.5
        conf = max(0.0, min(1.0, conf))
        if not title or not evidence:
            continue
        out["proposals"].append({
            "title": title,
            "evidence": evidence,
            "estimated_complexity": complexity,
            "confidence": conf,
        })
print(json.dumps(out))
PY
