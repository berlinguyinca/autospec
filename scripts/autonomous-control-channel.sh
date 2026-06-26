#!/usr/bin/env bash
# scripts/autonomous-control-channel.sh — cycle-boundary control-channel helper.
#
# Queries GitHub for open issues carrying any of the four reserved autospec
# control labels and prints a command decision token for each found label.
# Intended to be called once per outer-loop cycle, before dispatching the next
# issue.
#
# Reserved labels and their command tokens:
#   autospec:priority  →  DECISION:priority   (reorder this issue to front of queue)
#   autospec:steer     →  DECISION:steer      (emit issue body as directive)
#   autospec:pause     →  DECISION:pause      (write autonomous-pause.flag)
#   autospec:stop      →  DECISION:graceful-stop
#
# Output (stdout):
#   One line per found label, ordered by severity (stop > pause > steer > priority).
#   For each DECISION, the caller should process the first line and act on it.
#
#   Additional lines for steer:
#     STEER_ISSUE:<number>
#     DIRECTIVE:<url-encoded body, single line — newlines replaced with \n>
#     STEER_REMOVE_LABEL:<number>
#
#   Additional line for priority:
#     PRIORITY_ISSUE:<number>
#
# Side effects:
#   - :pause  → writes ~/.autospec/autonomous-pause.flag (atomic via temp+mv)
#   - :stop   → caller is expected to invoke autospec-stop.sh --graceful
#   - :steer  → caller should remove label autospec:steer from the issue
#
# Usage:
#   bash scripts/autonomous-control-channel.sh [--repo OWNER/REPO] [--state-dir DIR]
#
# Environment overrides (for testing):
#   AUTOSPEC_GH_CMD        — path to gh binary (default: gh from PATH)
#   AUTOSPEC_CONTROL_STATE_DIR — override ~/.autospec (for test isolation)
#
# Exit codes:
#   0  — success (zero or more decisions printed)
#   2  — usage error
#
# Bash safety rules:
#   - set -eu; no RETURN traps (they leak under set -u)
#   - if/then/fi for all one-sided conditionals (not `[ x ] && action`)
#   - jq: never interpolate host/user-derived values into test()/match();
#     use capture()+== for equality matching (see feedback_jq_test_regex_metachar_injection)

set -eu

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
RESERVED_LABELS="autospec:stop autospec:pause autospec:steer autospec:priority"

FLAG_DIR="${AUTOSPEC_CONTROL_STATE_DIR:-${HOME}/.autospec}"
PAUSE_FLAG="${FLAG_DIR}/autonomous-pause.flag"

GH="${AUTOSPEC_GH_CMD:-gh}"

REPO="${AUTOSPEC_REPO:-}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
warn()  { printf '[control-channel] WARN: %s\n' "$*" >&2; }

usage() {
    cat <<'EOF'
Usage: autonomous-control-channel.sh [--repo OWNER/REPO] [--state-dir DIR]

Query reserved autospec control labels and print a command decision token.

Options:
  --repo OWNER/REPO   Override the GitHub repo (default: detected from git remote).
  --state-dir DIR     Override ~/.autospec state directory (for testing).
  --help              Print this help.

Reserved labels (checked in severity order — stop is highest):
  autospec:stop      → DECISION:graceful-stop
  autospec:pause     → DECISION:pause  (writes autonomous-pause.flag)
  autospec:steer     → DECISION:steer  (emits DIRECTIVE:<body>)
  autospec:priority  → DECISION:priority

Output: one DECISION line per found label, highest severity first.
EOF
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo)       REPO="$2"; shift 2 ;;
        --state-dir)  FLAG_DIR="$2"; PAUSE_FLAG="${FLAG_DIR}/autonomous-pause.flag"; shift 2 ;;
        --help|-h)    usage; exit 0 ;;
        *)            printf 'autonomous-control-channel: unknown argument: %s\n' "$1" >&2; usage; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------
# Repo flag (optional — gh picks it up from git remote when omitted)
# ---------------------------------------------------------------------------
REPO_FLAG=""
if [ -n "$REPO" ]; then
    REPO_FLAG="--repo $REPO"
fi

# ---------------------------------------------------------------------------
# query_label LABEL — emit JSON array of matching open issues (number, body, title).
# Returns "[]" on any gh failure (fail-open: do not block the outer loop).
# ---------------------------------------------------------------------------
query_label() {
    local label="$1"
    # shellcheck disable=SC2086
    "$GH" issue list \
        --label "$label" \
        --state open \
        --limit 10 \
        --json number,title,body \
        $REPO_FLAG \
        2>/dev/null \
    || echo "[]"
}

# ---------------------------------------------------------------------------
# write_pause_flag — atomic write via temp+mv; idempotent.
# ---------------------------------------------------------------------------
write_pause_flag() {
    local stamp
    stamp="$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'unknown')"
    mkdir -p "$FLAG_DIR"
    local tmp
    tmp="$(mktemp "${FLAG_DIR}/.autonomous-pause.XXXXXX")"
    printf 'pause\n%s\n' "$stamp" > "$tmp"
    mv "$tmp" "$PAUSE_FLAG"
}

# ---------------------------------------------------------------------------
# emit_decision LABEL ISSUES_JSON — process one label and print decision line(s).
# ---------------------------------------------------------------------------
emit_decision() {
    local label="$1"
    local issues_json="$2"

    # Check if any issues were found: use jq length (safe — no host-derived interpolation).
    local count
    count="$(printf '%s' "$issues_json" | jq 'length' 2>/dev/null || echo 0)"

    if [ "$count" -eq 0 ]; then
        return 0
    fi

    case "$label" in
        "autospec:stop")
            printf 'DECISION:graceful-stop\n'
            ;;

        "autospec:pause")
            write_pause_flag
            printf 'DECISION:pause\n'
            ;;

        "autospec:steer")
            # Extract the first steer issue number and body.
            # Use jq capture()+== pattern: extract fields by position, no test() on dynamic values.
            local issue_number issue_body
            issue_number="$(printf '%s' "$issues_json" | jq -r '.[0].number // empty' 2>/dev/null || echo "")"
            issue_body="$(printf '%s' "$issues_json" | jq -r '.[0].body // empty' 2>/dev/null || echo "")"

            if [ -z "$issue_number" ]; then
                warn "steer: could not extract issue number from JSON"
                return 0
            fi

            # Encode body as a single line: replace newlines with literal \n.
            local body_oneline
            body_oneline="$(printf '%s' "$issue_body" | tr '\n' '|' | sed 's/|/\\n/g')"

            printf 'DECISION:steer\n'
            printf 'STEER_ISSUE:%s\n' "$issue_number"
            printf 'DIRECTIVE:%s\n' "$body_oneline"
            printf 'STEER_REMOVE_LABEL:%s\n' "$issue_number"
            ;;

        "autospec:priority")
            # Extract the first priority issue number.
            local issue_number
            issue_number="$(printf '%s' "$issues_json" | jq -r '.[0].number // empty' 2>/dev/null || echo "")"

            printf 'DECISION:priority\n'
            if [ -n "$issue_number" ]; then
                printf 'PRIORITY_ISSUE:%s\n' "$issue_number"
            fi
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Main: query each reserved label in severity order, emit decisions.
# ---------------------------------------------------------------------------
# Severity order: stop (highest) > pause > steer > priority (lowest).
for label in $RESERVED_LABELS; do
    issues_json="$(query_label "$label")"
    emit_decision "$label" "$issues_json"
done

exit 0
