#!/usr/bin/env bash
# scripts/sweep-area-dispatch.sh — parallel area dispatcher for autospec-sweep.
#
# Reads the 4 area definitions under skills/autospec-sweep/areas/ and dispatches
# each area's researcher in parallel via the harness-aware dispatcher
# (scripts/lib/autospec-harness-detect.sh). Aggregates the per-area JSON into a
# single sweep report at .autospec/sweep/area-findings.json.
#
# 3 areas reuse existing autospec-explore researchers; 1 area is the new
# dependency-health researcher (which also extends autospec-explore to 7).
#
# Usage:
#   bash scripts/sweep-area-dispatch.sh [--out PATH]
#
# Honors:
#   AUTOSPEC_REPO_ROOT  override repo root
#   AUTOSPEC_SWEEP_OUT  override output path

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
cd "$REPO_ROOT" || exit 1

OUT="${AUTOSPEC_SWEEP_OUT:-.autospec/sweep/area-findings.json}"
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT="$2"; shift 2 ;;
        *) shift ;;
    esac
done

mkdir -p "$(dirname "$OUT")"

AREAS_DIR="skills/autospec-sweep/areas"
RESEARCH_DIR="scripts/explore-research"

# Area → researcher mapping. Keep in sync with skills/autospec-sweep/areas/*.md.
declare -a AREAS=(
    "spec-vs-code-drift:$RESEARCH_DIR/spec-vs-code.sh"
    "docs-drift:scripts/dogfood-adapter-doc-drift.sh"
    "code-health:$RESEARCH_DIR/codebase-signals.sh"
    "dependency-health:$RESEARCH_DIR/dependency-health.sh"
)

# Verify all 4 area definition files exist.
for area_file in spec-vs-code-drift docs-drift code-health dependency-health; do
    if [ ! -f "$AREAS_DIR/$area_file.md" ]; then
        echo "ERROR: missing area definition: $AREAS_DIR/$area_file.md" >&2
        exit 2
    fi
done

TMPDIR_RUN="$(mktemp -d -t sweep-dispatch.XXXXXX)"
trap 'rm -rf "$TMPDIR_RUN"' EXIT

# Dispatch each area's researcher in parallel.
pids=()
for entry in "${AREAS[@]}"; do
    area="${entry%%:*}"
    script="${entry#*:}"
    out_file="$TMPDIR_RUN/$area.json"
    if [ -x "$script" ] || [ -f "$script" ]; then
        ( bash "$script" >"$out_file" 2>/dev/null \
            || printf '{"source":"%s","proposals":[],"error":"researcher failed"}' "$area" >"$out_file" ) &
        pids+=("$!")
    else
        printf '{"source":"%s","proposals":[],"error":"researcher missing"}' "$area" >"$out_file"
    fi
done

# Wait for all parallel researchers.
for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done

# Aggregate into a single report.
python3 - "$TMPDIR_RUN" "$OUT" <<'PY'
import json, os, sys, glob
tmp, out = sys.argv[1], sys.argv[2]
areas = {}
for f in sorted(glob.glob(os.path.join(tmp, "*.json"))):
    name = os.path.basename(f)[:-5]
    try:
        with open(f) as fh:
            areas[name] = json.load(fh)
    except Exception as e:
        areas[name] = {"source": name, "proposals": [], "error": str(e)}
report = {
    "schema": "autospec-sweep.area-findings.v1",
    "areas": areas,
    "summary": {
        "area_count": len(areas),
        "total_proposals": sum(len((a or {}).get("proposals", [])) for a in areas.values()),
    },
}
with open(out, "w") as fh:
    json.dump(report, fh, indent=2)
print(out)
PY
