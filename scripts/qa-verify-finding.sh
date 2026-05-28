#!/usr/bin/env bash
# qa-verify-finding.sh — verify a QA cluster finding still reproduces at
# current HEAD before letting the cluster emit it.
#
# Per-category cheap probes:
#   missing_function / missing_import → git grep for the symbol; symbol absent → KEEP
#   failing_test                       → run the test once; failing → KEEP
#   spec_mismatch                      → re-read both files at HEAD; claim still holds → KEEP
#   regression                         → newer commits touched the area → likely DROP
#
# Exit codes:
#   0 — finding still reproduces, KEEP it
#   1 — symptom no longer reproduces, DROP it
#   2 — usage / bad arguments

set -u

usage() {
    cat <<'EOF'
Usage: qa-verify-finding.sh --category <cat> [args...]

Categories and args:
  --category missing_function --symbol <name> [--file <path>]
  --category missing_import   --symbol <name> [--file <path>]
  --category failing_test     --test-cmd <cmd>
  --category spec_mismatch    --spec <spec-path> --impl <impl-path> --claim <text>
  --category regression       --commit <sha> [--file <path>]

Exit 0: finding still reproduces (KEEP).
Exit 1: symptom resolved (DROP).
Exit 2: bad usage.
EOF
}

if [ "$#" -eq 0 ] || [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    usage
    exit 2
fi

CATEGORY=""
SYMBOL=""
FILE=""
TEST_CMD=""
SPEC=""
IMPL=""
CLAIM=""
COMMIT=""
LINE=""
SYMPTOM=""
EMIT_RECORD=""
SUMMARY=""
EVIDENCE=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --category)     CATEGORY="$2";    shift 2 ;;
        --symbol)       SYMBOL="$2";      shift 2 ;;
        --file)         FILE="$2";        shift 2 ;;
        --test-cmd)     TEST_CMD="$2";    shift 2 ;;
        --spec)         SPEC="$2";        shift 2 ;;
        --impl)         IMPL="$2";        shift 2 ;;
        --claim)        CLAIM="$2";       shift 2 ;;
        --commit)       COMMIT="$2";      shift 2 ;;
        --line)         LINE="$2";        shift 2 ;;
        --symptom)      SYMPTOM="$2";     shift 2 ;;
        --emit-record)  EMIT_RECORD="$2"; shift 2 ;;
        --summary)      SUMMARY="$2";     shift 2 ;;
        --evidence)     EVIDENCE="$2";    shift 2 ;;
        *) echo "qa-verify-finding: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

if [ -z "$CATEGORY" ]; then
    echo "qa-verify-finding: --category is required" >&2
    usage
    exit 2
fi

# --emit-record mode (issue #647): run the per-category probe and, on KEEP,
# write a properly-formatted finding JSON to the target file containing
# verified_at, verified_by, verify_category, verify_args, summary, evidence.
# Exits with the same 0/1/2 contract as the bare probe so callers can branch.
if [ -n "$EMIT_RECORD" ]; then
    if ! command -v python3 >/dev/null 2>&1; then
        echo "qa-verify-finding: python3 required for --emit-record" >&2
        exit 2
    fi
    HEAD_SHA="$(git rev-parse HEAD 2>/dev/null || true)"
    if [ -z "$HEAD_SHA" ]; then
        echo "qa-verify-finding: --emit-record requires a git repo" >&2
        exit 2
    fi
    # Capture probe verdict by re-invoking ourselves without --emit-record.
    self="$0"
    probe_args=(--category "$CATEGORY")
    [ -n "$SYMBOL" ]   && probe_args+=(--symbol   "$SYMBOL")
    [ -n "$FILE" ]     && probe_args+=(--file     "$FILE")
    [ -n "$TEST_CMD" ] && probe_args+=(--test-cmd "$TEST_CMD")
    [ -n "$SPEC" ]     && probe_args+=(--spec     "$SPEC")
    [ -n "$IMPL" ]     && probe_args+=(--impl     "$IMPL")
    [ -n "$CLAIM" ]    && probe_args+=(--claim    "$CLAIM")
    [ -n "$COMMIT" ]   && probe_args+=(--commit   "$COMMIT")
    set +e
    bash "$self" "${probe_args[@]}" >/dev/null 2>&1
    probe_rc=$?
    set -e 2>/dev/null || true
    if [ "$probe_rc" -ne 0 ] && [ "$probe_rc" -ne 1 ]; then
        echo "qa-verify-finding: probe failed (rc=$probe_rc)" >&2
        exit 2
    fi
    HEAD="$HEAD_SHA" \
    CAT="$CATEGORY" \
    OUT="$EMIT_RECORD" \
    SUMMARY="$SUMMARY" \
    EVIDENCE="$EVIDENCE" \
    SYMBOL="$SYMBOL" \
    FILE="$FILE" \
    TEST_CMD="$TEST_CMD" \
    SPEC="$SPEC" \
    IMPL="$IMPL" \
    CLAIM="$CLAIM" \
    COMMIT="$COMMIT" \
    PROBE_RC="$probe_rc" \
    python3 - <<'PY'
import json, os
verify_args = {}
for k_env, k_out in [("SYMBOL","symbol"),("FILE","file"),("TEST_CMD","test-cmd"),
                     ("SPEC","spec"),("IMPL","impl"),("CLAIM","claim"),
                     ("COMMIT","commit")]:
    v = os.environ.get(k_env, "")
    if v:
        verify_args[k_out] = v
record = {
    "category":         os.environ["CAT"],
    "release_blocking": True,
    "status":           "fail" if os.environ["PROBE_RC"] == "0" else "resolved",
    "summary":          os.environ.get("SUMMARY", "") or "<no summary>",
    "evidence":         os.environ.get("EVIDENCE", "") or "<no evidence>",
    "verified_at":      os.environ["HEAD"],
    "verified_by":      "qa-verify-finding.sh",
    "verify_category":  os.environ["CAT"],
    "verify_args":      verify_args,
}
with open(os.environ["OUT"], "w") as f:
    json.dump(record, f, indent=2)
    f.write("\n")
PY
    # Forward the probe verdict to the caller (0 keep, 1 drop).
    exit "$probe_rc"
fi

case "$CATEGORY" in
    missing_function|missing_import)
        if [ -z "$SYMBOL" ]; then
            echo "qa-verify-finding: --symbol required for $CATEGORY" >&2
            exit 2
        fi
        if [ -n "$FILE" ] && [ -f "$FILE" ]; then
            if git grep -nE "$SYMBOL" -- "$FILE" >/dev/null 2>&1; then
                exit 1  # symbol present in file → resolved
            fi
            exit 0  # symbol still missing in file → keep finding
        fi
        if git grep -nE "$SYMBOL" >/dev/null 2>&1; then
            exit 1  # found anywhere → resolved
        fi
        exit 0
        ;;
    failing_test)
        if [ -z "$TEST_CMD" ]; then
            echo "qa-verify-finding: --test-cmd required for failing_test" >&2
            exit 2
        fi
        # Run the test once; if it still fails, finding reproduces.
        if bash -c "$TEST_CMD" >/dev/null 2>&1; then
            exit 1  # test passes → resolved
        fi
        exit 0  # test still fails → keep finding
        ;;
    spec_mismatch)
        if [ -z "$SPEC" ] || [ -z "$IMPL" ] || [ -z "$CLAIM" ]; then
            echo "qa-verify-finding: --spec, --impl, --claim required for spec_mismatch" >&2
            exit 2
        fi
        # Claim holds if spec contains the claim text but impl does NOT.
        if [ ! -f "$SPEC" ] || [ ! -f "$IMPL" ]; then
            exit 0  # missing file → can't refute, keep
        fi
        if grep -qF "$CLAIM" "$SPEC" && ! grep -qF "$CLAIM" "$IMPL"; then
            exit 0  # still mismatched
        fi
        exit 1  # claim now reflected in impl (or absent from spec) → resolved
        ;;
    regression)
        if [ -z "$COMMIT" ]; then
            echo "qa-verify-finding: --commit required for regression" >&2
            exit 2
        fi
        # If newer commits on HEAD touched the area, likely resolved.
        local_range_files=()
        if [ -n "$FILE" ]; then
            local_range_files=(-- "$FILE")
        fi
        if ! git cat-file -e "$COMMIT" 2>/dev/null; then
            exit 0  # named commit no longer exists in history → keep
        fi
        newer=$(git log --oneline "$COMMIT"..HEAD "${local_range_files[@]}" 2>/dev/null | wc -l | tr -d ' ')
        if [ "${newer:-0}" -gt 0 ]; then
            exit 1  # newer commits touched the area → likely resolved
        fi
        exit 0
        ;;
    *)
        echo "qa-verify-finding: unknown category: $CATEGORY" >&2
        usage
        exit 2
        ;;
esac
