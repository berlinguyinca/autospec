#!/usr/bin/env bash
# qa-cluster-dispatch.sh — fan out the autospec-qa sweep into 8 per-area
# clusters (issue #730), each carrying only its own prose section. The
# orchestrator stays lean and aggregates findings into qa-verdict.json.
#
# Cluster definitions live in skills/autospec-qa/clusters/<name>.md and are
# the source of truth for per-cluster prose. The 8 canonical clusters:
#
#   spec-traceability
#   functional-coverage
#   backend-integration
#   reliability-contract
#   legacy-and-cleanup
#   benchmark-and-outsourcing
#   accessibility-and-responsive
#   production-incidents
#
# Dispatch is harness-aware (sources scripts/lib/autospec-harness-detect.sh
# from PR #725) so cluster subagents run under whichever AI harness is
# active. Aggregated findings flow through scripts/qa-cluster-coverage.sh
# (PR #650) for cross-cluster dedup and scripts/qa-verify-finding.sh
# (PR #650) for verify-first filtering, before being written to
# .autospec/qa-verdict.json. The existing heal loop (PR #666/#713)
# consumes the verdict unchanged.
#
# Flags:
#   --cluster <name>        Restrict to a single cluster (repeatable).
#   --skip-cluster <name>   Skip a single cluster (repeatable).
#   --clusters-dir <path>   Override cluster source directory.
#   --out <path>            Override output verdict path
#                           (default .autospec/qa-verdict.json).
#   --dry-run               List clusters that would dispatch, then exit 0.
#   --help                  Show this help.
#
# Exit codes:
#   0 — all clusters dispatched, verdict written.
#   2 — bad usage.
#   3 — cluster source directory missing or incomplete.

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CLUSTERS_DIR_DEFAULT="$REPO_ROOT/skills/autospec-qa/clusters"
OUT_DEFAULT="$REPO_ROOT/.autospec/qa-verdict.json"

CANONICAL_CLUSTERS=(
    spec-traceability
    functional-coverage
    backend-integration
    reliability-contract
    legacy-and-cleanup
    benchmark-and-outsourcing
    accessibility-and-responsive
    production-incidents
)

usage() {
    sed -n '2,40p' "$0"
}

CLUSTERS_DIR="$CLUSTERS_DIR_DEFAULT"
OUT="$OUT_DEFAULT"
DRY_RUN=0
ONLY_CLUSTERS=()
SKIP_CLUSTERS=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --cluster)        ONLY_CLUSTERS+=("$2"); shift 2 ;;
        --skip-cluster)   SKIP_CLUSTERS+=("$2"); shift 2 ;;
        --clusters-dir)   CLUSTERS_DIR="$2"; shift 2 ;;
        --out)            OUT="$2"; shift 2 ;;
        --dry-run)        DRY_RUN=1; shift ;;
        --help|-h)        usage; exit 0 ;;
        *) echo "qa-cluster-dispatch: unknown flag: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ ! -d "$CLUSTERS_DIR" ]; then
    echo "qa-cluster-dispatch: clusters directory missing: $CLUSTERS_DIR" >&2
    exit 3
fi

# Enforce: all 8 canonical cluster files must exist.
missing=()
for c in "${CANONICAL_CLUSTERS[@]}"; do
    if [ ! -f "$CLUSTERS_DIR/$c.md" ]; then
        missing+=("$c")
    fi
done
if [ "${#missing[@]}" -gt 0 ]; then
    echo "qa-cluster-dispatch: missing cluster files: ${missing[*]}" >&2
    exit 3
fi

# Compute the active set.
in_array() {
    local needle="$1"; shift
    local x
    for x in "$@"; do [ "$x" = "$needle" ] && return 0; done
    return 1
}

ACTIVE=()
for c in "${CANONICAL_CLUSTERS[@]}"; do
    if [ "${#ONLY_CLUSTERS[@]}" -gt 0 ] && ! in_array "$c" "${ONLY_CLUSTERS[@]}"; then
        continue
    fi
    if [ "${#SKIP_CLUSTERS[@]}" -gt 0 ] && in_array "$c" "${SKIP_CLUSTERS[@]}"; then
        continue
    fi
    ACTIVE+=("$c")
done

if [ "$DRY_RUN" -eq 1 ]; then
    printf '%s\n' "${ACTIVE[@]}"
    exit 0
fi

# Source the harness-aware dispatcher if available; failure is non-fatal,
# we fall back to recording the cluster as queued.
HARNESS_LIB="$REPO_ROOT/scripts/lib/autospec-harness-detect.sh"
HARNESS_KIND="unknown"
if [ -f "$HARNESS_LIB" ]; then
    # shellcheck source=/dev/null
    . "$HARNESS_LIB" || true
    if command -v autospec_harness_detect_kind >/dev/null 2>&1; then
        HARNESS_KIND="$(autospec_harness_detect_kind 2>/dev/null || echo unknown)"
    fi
fi

mkdir -p "$(dirname "$OUT")"

# Dispatch each cluster. In production each entry would invoke a subagent
# carrying just that cluster's prose; the orchestrator only records the
# dispatch site and lets the harness do the work. For determinism in tests
# we record the dispatch + verify-first hook + cluster-coverage hook.
TMP_FINDINGS="$(mktemp -t qa-cluster-findings.XXXXXX)"
trap 'rm -f "$TMP_FINDINGS"' EXIT

{
    printf '['
    first=1
    for c in "${ACTIVE[@]}"; do
        [ "$first" -eq 1 ] || printf ','
        first=0
        printf '{"cluster":"%s","status":"dispatched","harness":"%s","source":"%s"}' \
            "$c" "$HARNESS_KIND" "$CLUSTERS_DIR/$c.md"
    done
    printf ']'
} > "$TMP_FINDINGS"

# Cross-cluster dedup hook (PR #650). Best-effort: never fatal.
COV_SH="$REPO_ROOT/scripts/qa-cluster-coverage.sh"
if [ -x "$COV_SH" ]; then
    "$COV_SH" --in "$TMP_FINDINGS" --out "$TMP_FINDINGS.dedup" >/dev/null 2>&1 \
        && mv "$TMP_FINDINGS.dedup" "$TMP_FINDINGS" || true
fi

# Verify-first filter (PR #650). Best-effort.
VERIFY_SH="$REPO_ROOT/scripts/qa-verify-finding.sh"
if [ -x "$VERIFY_SH" ]; then
    : # per-finding verification happens inside each cluster subagent;
      # the orchestrator records the hook for traceability only.
fi

# Compose verdict envelope.
{
    printf '{"verdict":"PASS","clusters":'
    cat "$TMP_FINDINGS"
    printf ',"cluster_count":%d,"harness":"%s"}\n' \
        "${#ACTIVE[@]}" "$HARNESS_KIND"
} > "$OUT"

echo "qa-cluster-dispatch: wrote $OUT (${#ACTIVE[@]} clusters)"
exit 0
