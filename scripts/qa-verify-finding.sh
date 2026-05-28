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

while [ "$#" -gt 0 ]; do
    case "$1" in
        --category) CATEGORY="$2"; shift 2 ;;
        --symbol)   SYMBOL="$2";   shift 2 ;;
        --file)     FILE="$2";     shift 2 ;;
        --test-cmd) TEST_CMD="$2"; shift 2 ;;
        --spec)     SPEC="$2";     shift 2 ;;
        --impl)     IMPL="$2";     shift 2 ;;
        --claim)    CLAIM="$2";    shift 2 ;;
        --commit)   COMMIT="$2";   shift 2 ;;
        --line)     LINE="$2";     shift 2 ;;
        --symptom)  SYMPTOM="$2";  shift 2 ;;
        *) echo "qa-verify-finding: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

if [ -z "$CATEGORY" ]; then
    echo "qa-verify-finding: --category is required" >&2
    usage
    exit 2
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
