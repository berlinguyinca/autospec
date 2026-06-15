#!/usr/bin/env bash
# scripts/explore-research-cycle.sh — autospec-explore research cycle aggregator.
#
# Runs the enabled deterministic researchers in parallel, aggregates their
# proposals, deduplicates by normalized title, ranks by weighted score,
# filters out proposals matching titles created in the last 7 days, and
# caps output at --max-issues-per-round.
#
# Implements the "Research cycle contract" + "Per-researcher contracts"
# rows 1-4 in docs/specs/2026-05-29-autospec-explore-design.md.
#
# Output: JSON to stdout with shape:
#   { "round": "<iso-date>",
#     "proposals_total": N,
#     "proposals_after_dedup": N,
#     "proposals_after_recent_filter": N,
#     "proposals": [ ...top --max-issues-per-round... ] }
#
# Each proposal carries: title, evidence, estimated_complexity, confidence,
# source, score.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESEARCH_DIR="${AUTOSPEC_RESEARCH_DIR:-$SCRIPT_DIR/explore-research}"

usage() {
    cat <<'EOF'
Usage: explore-research-cycle.sh [options]

Options:
  --max-issues-per-round N     Cap final proposals (default 5).
  --research-sources LIST      Comma-separated subset of:
                                 spec-vs-code,prior-reports,codebase-signals,open-issues
                               Default: all 4.
  --ledger PATH                Outcome ledger to derive dynamic source weights
                               from (passed to explore-source-weights.sh). When
                               omitted, $AUTOSPEC_EXPLORE_LEDGER is used, then the
                               weights script's own default. With no ledger the
                               canonical static priors are used (unchanged).
  --out PATH                   Write JSON to PATH (atomic) instead of stdout.
  -h, --help                   Print this help.

Env:
  AUTOSPEC_REPO_ROOT             Repo root (default: git rev-parse).
  AUTOSPEC_RESEARCH_DIR          Researcher directory.
  AUTOSPEC_EXPLORE_LEDGER        Default outcome ledger path (overridden by
                                 --ledger).
  AUTOSPEC_EXPLORE_WEIGHTS_BIN   Explicit path to explore-source-weights.sh
                                 (overrides sibling/repo resolution; testing).
  AUTOSPEC_SCRIPTS_DIR           Shared scripts dir searched for the weights bin.
  AUTOSPEC_TEST_ISSUES_JSON      Inject fake gh issue list (testing).
  AUTOSPEC_TEST_RECENT_TITLES    Newline-separated titles created in last 7d
                                 (testing — bypasses gh search).
EOF
}

MAX_ISSUES=5
SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues"
OUT=""
LEDGER="${AUTOSPEC_EXPLORE_LEDGER:-}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --max-issues-per-round) MAX_ISSUES="$2"; shift 2 ;;
        --research-sources)     SOURCES="$2"; shift 2 ;;
        --ledger)               LEDGER="$2"; shift 2 ;;
        --out)                  OUT="$2"; shift 2 ;;
        -h|--help)              usage; exit 0 ;;
        *) echo "explore-research-cycle: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

# Resolve explore-source-weights.sh defensively. Order:
#   1. $AUTOSPEC_EXPLORE_WEIGHTS_BIN (explicit override, e.g. tests)
#   2. $AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh
#   3. sibling of this script
#   4. <repo>/skills/autospec-shared/scripts/explore-source-weights.sh
# Prints the resolved path (may be empty / nonexistent — caller checks -x).
_resolve_weights_bin() {
    if [ -n "${AUTOSPEC_EXPLORE_WEIGHTS_BIN:-}" ]; then
        printf '%s\n' "$AUTOSPEC_EXPLORE_WEIGHTS_BIN"; return 0
    fi
    if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "$AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh" ]; then
        printf '%s\n' "$AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh"; return 0
    fi
    if [ -f "$SCRIPT_DIR/explore-source-weights.sh" ]; then
        printf '%s\n' "$SCRIPT_DIR/explore-source-weights.sh"; return 0
    fi
    if [ -f "$REPO_ROOT/skills/autospec-shared/scripts/explore-source-weights.sh" ]; then
        printf '%s\n' "$REPO_ROOT/skills/autospec-shared/scripts/explore-source-weights.sh"; return 0
    fi
    printf '\n'
}

cd "$REPO_ROOT" || { echo '{"proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo "explore-research-cycle: python3 required" >&2
    exit 2
fi

# Run each enabled researcher in parallel, write to a sibling tmp file.
work_dir="$(mktemp -d -t explore-cycle.XXXXXX)"
trap 'rm -rf "$work_dir"' EXIT

pids=()
IFS=','
for src in $SOURCES; do
    src="$(printf '%s' "$src" | tr -d ' ')"
    [ -z "$src" ] && continue
    script="$RESEARCH_DIR/$src.sh"
    if [ ! -f "$script" ]; then
        echo '{"source":"'"$src"'","proposals":[],"error":"missing_script"}' \
            > "$work_dir/$src.json"
        continue
    fi
    (
        bash "$script" > "$work_dir/$src.json" 2>"$work_dir/$src.err" \
            || echo '{"source":"'"$src"'","proposals":[],"error":"researcher_failed"}' \
                 > "$work_dir/$src.json"
    ) &
    pids+=("$!")
done
unset IFS

# Wait for all background researchers.
for p in "${pids[@]:-}"; do
    [ -n "$p" ] && wait "$p" 2>/dev/null || true
done

# Gather recent titles (last 7 days) to filter against. Allow injection.
recent_titles_file="$work_dir/recent.txt"
if [ -n "${AUTOSPEC_TEST_RECENT_TITLES:-}" ]; then
    printf '%s\n' "$AUTOSPEC_TEST_RECENT_TITLES" > "$recent_titles_file"
elif command -v gh >/dev/null 2>&1; then
    # Last 7 days = created:>=YYYY-MM-DD
    since="$(python3 -c 'import datetime; print((datetime.date.today() - datetime.timedelta(days=7)).isoformat())')"
    gh issue list --search "created:>=$since" --state all --limit 200 \
        --json title 2>/dev/null \
        | python3 -c 'import json,sys; [print(t["title"]) for t in json.load(sys.stdin)]' \
        > "$recent_titles_file" 2>/dev/null || : > "$recent_titles_file"
else
    : > "$recent_titles_file"
fi

# Compute dynamic, ledger-derived source weights (best-effort). The weights
# script emits {source: weight} JSON. With no/empty ledger it emits the
# canonical priors that EXACTLY equal DEFAULT_SRC_WEIGHTS below, so behavior is
# byte-identical when there is no learning yet. Any failure -> empty -> fallback.
WEIGHTS_JSON=""
weights_bin="$(_resolve_weights_bin)"
if [ -n "$weights_bin" ] && [ -x "$weights_bin" ]; then
    WEIGHTS_JSON="$("$weights_bin" --json ${LEDGER:+--ledger "$LEDGER"} 2>/dev/null || true)"
fi
export WEIGHTS_JSON

# Aggregate, dedup, rank, filter, cap.
WORK_DIR="$work_dir" \
MAX_ISSUES="$MAX_ISSUES" \
RECENT_FILE="$recent_titles_file" \
python3 - <<'PY' > "$work_dir/final.json"
import json, os, re, glob, datetime

work     = os.environ["WORK_DIR"]
cap      = int(os.environ["MAX_ISSUES"])
recent_f = os.environ["RECENT_FILE"]

# Static source priors per the spec — the defensive fallback used verbatim when
# no dynamic weights are available (no ledger / weights script absent / broken).
DEFAULT_SRC_WEIGHTS = {
    "spec-vs-code":     1.0,
    "prior-reports":    0.9,
    "codebase-signals": 0.7,
    "open-issues":      0.6,
    "source-analysis":  0.5,
    "internet":         0.4,
}
# Overlay dynamic ledger-derived weights when present; otherwise fall back to
# the static table. The overlay is a merge so unknown/missing sources retain
# their static prior, guaranteeing parity when WEIGHTS_JSON mirrors the priors.
_wj = os.environ.get("WEIGHTS_JSON", "").strip()
if _wj:
    try:
        SRC_WEIGHTS = {**DEFAULT_SRC_WEIGHTS, **json.loads(_wj)}
    except Exception:
        SRC_WEIGHTS = DEFAULT_SRC_WEIGHTS
else:
    SRC_WEIGHTS = DEFAULT_SRC_WEIGHTS
COMPLEXITY = {"small": 1.0, "medium": 2.0, "large": 4.0}

def normalize_title(t):
    s = t.lower()
    # Strip leading conventional-commit prefix.
    s = re.sub(r'^\s*(feat|fix|chore|docs|test|refactor|perf|track|ci)\s*:\s*', '', s)
    s = re.sub(r'[^a-z0-9 ]+', ' ', s)
    s = re.sub(r'\s+', ' ', s).strip()
    # Drop stopwords for normalization (keep verb + subject signal).
    return s[:120]

all_props = []
total = 0
for f in sorted(glob.glob(os.path.join(work, "*.json"))):
    if os.path.basename(f) == "final.json":
        continue
    try:
        with open(f, 'r', encoding='utf-8') as fh:
            data = json.load(fh)
    except Exception:
        continue
    src = data.get("source", "unknown")
    for p in data.get("proposals", []) or []:
        if not isinstance(p, dict): continue
        title = (p.get("title") or "").strip()
        if not title: continue
        comp = (p.get("estimated_complexity") or "medium").lower()
        try:
            conf = float(p.get("confidence", 0.5))
        except Exception:
            conf = 0.5
        weight = SRC_WEIGHTS.get(src, 0.5)
        cscale = COMPLEXITY.get(comp, 2.0)
        score = conf * weight / cscale
        all_props.append({
            "title": title,
            "evidence": p.get("evidence",""),
            "estimated_complexity": comp,
            "confidence": conf,
            "source": src,
            "score": round(score, 4),
        })
        total += 1

# Dedup by normalized title — keep highest score.
by_norm = {}
for p in all_props:
    n = normalize_title(p["title"])
    if n not in by_norm or p["score"] > by_norm[n]["score"]:
        by_norm[n] = p
deduped = list(by_norm.values())

# Constitution gate (deterministic rules D1 evidence + D2 confidence floor).
# Keep this byte-aligned with explore-constitution.sh --filter. Floor default 0.3
# reproduces prior behavior for well-formed proposals (all carry evidence and
# confidence >= 0.4); it drops empty-evidence and low-confidence noise.
try:
    _floor = float(os.environ.get("AUTOSPEC_EXPLORE_MIN_CONFIDENCE", "0.3") or "0.3")
except Exception:
    _floor = 0.3
constitutional = [p for p in deduped
                  if str(p.get("evidence", "")).strip() != ""
                  and p.get("confidence", 0) >= _floor]

# Filter against recent titles.
recent_norms = set()
try:
    with open(recent_f, 'r', encoding='utf-8') as fh:
        for line in fh:
            n = normalize_title(line.strip())
            if n:
                recent_norms.add(n)
except FileNotFoundError:
    pass

filtered = [p for p in constitutional if normalize_title(p["title"]) not in recent_norms]

# Rank descending by score; cap.
filtered.sort(key=lambda p: p["score"], reverse=True)
final = filtered[:cap]

out = {
    "round": datetime.date.today().isoformat(),
    "proposals_total": total,
    "proposals_after_dedup": len(deduped),
    "proposals_after_constitution": len(constitutional),
    "proposals_after_recent_filter": len(filtered),
    "proposals": final,
}
print(json.dumps(out, indent=2))
PY

if [ -n "$OUT" ]; then
    # Atomic write.
    case "$OUT" in
        /*) abs="$OUT" ;;
        *)  abs="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")" ;;
    esac
    mv "$work_dir/final.json" "$abs.tmp"
    mv "$abs.tmp" "$abs"
else
    cat "$work_dir/final.json"
fi
