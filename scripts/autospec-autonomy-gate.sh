#!/usr/bin/env bash
# autospec-autonomy-gate.sh — guardrails for /autospec --autonomous.
#
# Even in autonomous mode, surface a confirmation when one of these triggers:
#   1. Destructive remote actions (force-push to protected, repo delete, archive,
#      release delete, mass label changes, prod DB writes).
#   2. Out-of-scope file changes (planned files extend beyond Goal +
#      Implementation outline scope).
#   3. Cost gate (tokens > AUTOSPEC_AUTONOMOUS_TOKEN_CAP).
#   4. Existing autonomy-scope rules from
#      feedback_autospec_autonomy_scope.md remain in force.
#
# Exit codes:
#   0 — autonomous OK, proceed without asking
#   1 — surface confirmation to the operator ("ask anyway")
#   2 — error in invocation
#
# Usage:
#   autospec-autonomy-gate.sh --check destructive|out-of-scope|cost|all \
#     [--tokens N] [--files "<space-separated list>"] \
#     [--scope "<space-separated allowed prefixes>"] [--intent "<text>"]

set -eu

CHECK=""
TOKENS=""
FILES=""
SCOPE=""
INTENT=""

TOKEN_CAP="${AUTOSPEC_AUTONOMOUS_TOKEN_CAP:-500000}"

usage() {
    cat <<'EOF'
Usage: autospec-autonomy-gate.sh --check destructive|out-of-scope|cost|all [opts]

Options:
  --check WHICH      Which guardrail(s) to evaluate.
  --tokens N         Estimated total tokens (for --check cost|all).
  --files "LIST"     Space-separated planned file paths.
  --scope  "LIST"    Space-separated allowed scope prefixes.
  --intent "TEXT"    Free-form intent text scanned for destructive verbs.

Env:
  AUTOSPEC_AUTONOMOUS_TOKEN_CAP  (default 500000)

Exit 0 = autonomous OK; 1 = ask anyway; 2 = invocation error.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --check)   CHECK="${2:-}"; shift 2 ;;
        --issues)  shift 2 ;; # Backward-compatible no-op: issue counts are uncapped.
        --tokens)  TOKENS="${2:-}"; shift 2 ;;
        --files)   FILES="${2:-}"; shift 2 ;;
        --scope)   SCOPE="${2:-}"; shift 2 ;;
        --intent)  INTENT="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ -n "$CHECK" ] || { echo "missing --check" >&2; exit 2; }

ask=0
reason=""

check_destructive() {
    # Scan --intent for destructive remote verbs.
    local hay
    hay="$(printf '%s' "${INTENT:-}" | tr '[:upper:]' '[:lower:]')"
    # Patterns chosen per spec body.
    local patterns='gh repo archive|gh repo delete|gh release delete|git push --force|force-push|force push|delete all branches|mass label|prod db write|production database|drop database|rm -rf /|truncate table'
    if printf '%s' "$hay" | grep -qE "$patterns"; then
        ask=1
        reason="destructive remote action detected in intent"
        return
    fi
}

check_out_of_scope() {
    # Each file in FILES must start with at least one prefix in SCOPE.
    [ -n "$FILES" ] || return 0
    [ -n "$SCOPE" ] || { ask=1; reason="files planned but no scope provided"; return; }
    local f matched
    for f in $FILES; do
        matched=0
        for p in $SCOPE; do
            case "$f" in
                "$p"*) matched=1; break ;;
            esac
        done
        if [ "$matched" -eq 0 ]; then
            ask=1
            reason="out-of-scope file: $f"
            return
        fi
    done
}

check_cost() {
    if [ -n "$TOKENS" ]; then
        if [ "$TOKENS" -gt "$TOKEN_CAP" ] 2>/dev/null; then
            ask=1
            reason="token estimate $TOKENS exceeds AUTOSPEC_AUTONOMOUS_TOKEN_CAP=$TOKEN_CAP"
            return
        fi
    fi
}

case "$CHECK" in
    destructive)   check_destructive ;;
    out-of-scope)  check_out_of_scope ;;
    cost)          check_cost ;;
    all)           check_destructive
                   [ "$ask" -eq 0 ] && check_out_of_scope
                   [ "$ask" -eq 0 ] && check_cost ;;
    *) echo "unknown --check: $CHECK" >&2; exit 2 ;;
esac

if [ "$ask" -eq 1 ]; then
    printf 'autonomy-gate: ASK — %s\n' "$reason"
    exit 1
fi
printf 'autonomy-gate: OK (--check %s)\n' "$CHECK"
exit 0
