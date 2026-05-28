#!/usr/bin/env bash
# qa-cluster-coverage.sh — cross-cluster coverage ledger for autospec-qa.
#
# Atomic append-and-read of .autospec/qa-cluster-coverage.json so parallel
# QA cluster agents do not re-emit findings on the same
# (file, spec_section, rule_id) tuple. The first cluster to claim a tuple
# wins; subsequent claims fail and the caller logs `cluster_skip_dedup`.
#
# Commands:
#   claim --cluster <id> --file <path> --section <section> --rule <rule_id>
#       Try to claim the tuple. Exit 0 on success, 1 if already claimed.
#   list
#       Print every claimed tuple as JSON Lines.
#   clear
#       Truncate the ledger.
#
# Env:
#   AUTOSPEC_QA_COVERAGE_FILE — override ledger path (default
#                              .autospec/qa-cluster-coverage.json).

set -u

LEDGER="${AUTOSPEC_QA_COVERAGE_FILE:-.autospec/qa-cluster-coverage.json}"

usage() {
    cat <<'EOF'
Usage:
  qa-cluster-coverage.sh claim --cluster <id> --file <path> --section <section> --rule <rule_id>
  qa-cluster-coverage.sh list
  qa-cluster-coverage.sh clear

Env:
  AUTOSPEC_QA_COVERAGE_FILE  override ledger path
EOF
}

with_lock() {
    # Atomic critical section. Prefer flock when available; fall back to
    # mkdir-as-lock so this works on macOS where flock is not standard.
    local cmd_fn="$1"
    mkdir -p "$(dirname "$LEDGER")"
    if command -v flock >/dev/null 2>&1; then
        local lockfile="$LEDGER.lock"
        : > "$lockfile" 2>/dev/null || true
        ( flock 9; "$cmd_fn" ) 9>>"$lockfile"
        return $?
    fi
    local lockdir="$LEDGER.lockd"
    local tries=0
    while ! mkdir "$lockdir" 2>/dev/null; do
        tries=$((tries + 1))
        if [ "$tries" -gt 200 ]; then
            echo "qa-cluster-coverage: lock timeout on $lockdir" >&2
            return 2
        fi
        sleep 0.05
    done
    trap 'rmdir "$lockdir" 2>/dev/null' EXIT
    "$cmd_fn"
    local rc=$?
    rmdir "$lockdir" 2>/dev/null
    trap - EXIT
    return $rc
}

_do_claim() {
    [ -f "$LEDGER" ] || : > "$LEDGER"
    # Look for existing matching tuple.
    if grep -F -q "\"file\":\"$FILE\",\"section\":\"$SECTION\",\"rule\":\"$RULE\"" "$LEDGER" 2>/dev/null; then
        return 1
    fi
    printf '{"cluster":"%s","file":"%s","section":"%s","rule":"%s"}\n' \
        "$CLUSTER" "$FILE" "$SECTION" "$RULE" >> "$LEDGER"
    return 0
}

_do_list() {
    if [ -f "$LEDGER" ]; then
        cat "$LEDGER"
    fi
    return 0
}

_do_clear() {
    : > "$LEDGER"
    return 0
}

CMD="${1:-}"
if [ -z "$CMD" ] || [ "$CMD" = "--help" ] || [ "$CMD" = "-h" ]; then
    usage
    exit 2
fi
shift || true

case "$CMD" in
    claim)
        CLUSTER=""; FILE=""; SECTION=""; RULE=""
        while [ "$#" -gt 0 ]; do
            case "$1" in
                --cluster) CLUSTER="$2"; shift 2 ;;
                --file)    FILE="$2";    shift 2 ;;
                --section) SECTION="$2"; shift 2 ;;
                --rule)    RULE="$2";    shift 2 ;;
                *) echo "qa-cluster-coverage: unknown arg: $1" >&2; usage; exit 2 ;;
            esac
        done
        if [ -z "$CLUSTER" ] || [ -z "$FILE" ] || [ -z "$SECTION" ] || [ -z "$RULE" ]; then
            echo "qa-cluster-coverage: claim requires --cluster --file --section --rule" >&2
            exit 2
        fi
        export CLUSTER FILE SECTION RULE
        with_lock _do_claim
        exit $?
        ;;
    list)
        with_lock _do_list
        exit $?
        ;;
    clear)
        with_lock _do_clear
        exit $?
        ;;
    *)
        echo "qa-cluster-coverage: unknown command: $CMD" >&2
        usage
        exit 2
        ;;
esac
