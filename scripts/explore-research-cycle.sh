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
  --out PATH                   Write JSON to PATH (atomic) instead of stdout.
  -h, --help                   Print this help.

Env:
  AUTOSPEC_REPO_ROOT             Repo root (default: git rev-parse).
  AUTOSPEC_RESEARCH_DIR          Researcher directory.
  AUTOSPEC_TEST_ISSUES_JSON      Inject fake gh issue list (testing).
  AUTOSPEC_TEST_RECENT_TITLES    Newline-separated titles created in last 7d
                                 (testing — bypasses gh search).
EOF
}

MAX_ISSUES=5
SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues"
OUT=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --max-issues-per-round) MAX_ISSUES="$2"; shift 2 ;;
        --research-sources)     SOURCES="$2"; shift 2 ;;
        --out)                  OUT="$2"; shift 2 ;;
        -h|--help)              usage; exit 0 ;;
        *) echo "explore-research-cycle: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

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

# Aggregate, dedup, rank, filter, cap.
WORK_DIR="$work_dir" \
MAX_ISSUES="$MAX_ISSUES" \
RECENT_FILE="$recent_titles_file" \
python3 - <<'PY' > "$work_dir/final.json"
import json, os, re, glob, datetime

work     = os.environ["WORK_DIR"]
cap      = int(os.environ["MAX_ISSUES"])
recent_f = os.environ["RECENT_FILE"]

# Source weights per the spec.
SRC_WEIGHTS = {
    "spec-vs-code":     1.0,
    "prior-reports":    0.9,
    "codebase-signals": 0.7,
    "open-issues":      0.6,
    "source-analysis":  0.5,
    "internet":         0.4,
}
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

filtered = [p for p in deduped if normalize_title(p["title"]) not in recent_norms]

# Rank descending by score; cap.
filtered.sort(key=lambda p: p["score"], reverse=True)
final = filtered[:cap]

out = {
    "round": datetime.date.today().isoformat(),
    "proposals_total": total,
    "proposals_after_dedup": len(deduped),
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
