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
#   autospec:promote   →  DECISION:promote    (trusted actor only — merge roll-up + reset)
#   autospec:discard   →  DECISION:discard    (close roll-up, delete branch, reopen issues)
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
#   Additional lines for promote (only when the issue author is a trusted
#   actor per safety.issue_intent_gate.trusted_actors — otherwise refused):
#     PROMOTE_ISSUE:<number>
#
#   Additional line for an untrusted promote attempt (refused, fail closed):
#     DECISION:promote-refused
#     PROMOTE_ISSUE:<number>
#
#   Additional line for discard:
#     DISCARD_ISSUE:<number>
#
# Side effects:
#   - :pause    → writes ~/.autospec/autonomous-pause.flag (atomic via temp+mv)
#   - :stop     → caller is expected to invoke autospec-stop.sh --graceful
#   - :steer    → caller should remove label autospec:steer from the issue
#   - :promote  → caller (conductor) merges the roll-up PR and runs
#                 `autonomous-integration-branch.sh reset` when trusted;
#                 refused (no action) when the issue author is untrusted
#   - :discard  → caller (conductor) closes the roll-up PR, deletes the
#                 integration branch, and reopens its issues
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
RESERVED_LABELS="autospec:stop autospec:pause autospec:steer autospec:priority autospec:recalibrate-persona autospec:promote autospec:discard"

FLAG_DIR="${AUTOSPEC_CONTROL_STATE_DIR:-${HOME}/.autospec}"
PAUSE_FLAG="${FLAG_DIR}/autonomous-pause.flag"
RECALIBRATE_FLAG="${FLAG_DIR}/persona-recalibrate.flag"

GH="${AUTOSPEC_GH_CMD:-gh}"

REPO="${AUTOSPEC_REPO:-}"

# ---------------------------------------------------------------------------
# Runtime config (for the promote trusted-actor gate). Sourced defensively —
# a missing helper leaves autospec_runtime_config_get undefined and the
# trusted-actor check below fails closed (empty trusted list → refused).
# ---------------------------------------------------------------------------
CONTROL_CHANNEL_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "${CONTROL_CHANNEL_SCRIPT_DIR}/autospec-runtime-config.sh" ]; then
    # shellcheck source=./autospec-runtime-config.sh
    . "${CONTROL_CHANNEL_SCRIPT_DIR}/autospec-runtime-config.sh"
elif [ -f "${HOME}/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "${HOME}/.autospec/scripts/autospec-runtime-config.sh"
fi

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
  autospec:promote   → DECISION:promote (trusted actor only) or DECISION:promote-refused
  autospec:discard   → DECISION:discard

Output: one DECISION line per found label, highest severity first.
EOF
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo)       REPO="$2"; shift 2 ;;
        --state-dir)  FLAG_DIR="$2"; PAUSE_FLAG="${FLAG_DIR}/autonomous-pause.flag"; RECALIBRATE_FLAG="${FLAG_DIR}/persona-recalibrate.flag"; shift 2 ;;
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
        --json number,title,body,author \
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
# control_channel_trusted_logins — newline-separated logins from
# safety.issue_intent_gate.trusted_actors, or empty on missing/invalid config.
# Mirrors autonomous-provenance.sh's trusted_actor_logins() parsing
# convention (same config key, same jq shape) so `promote` and the
# provenance resolver agree on who counts as trusted.
# ---------------------------------------------------------------------------
control_channel_trusted_logins() {
    local raw
    if ! command -v autospec_runtime_config_get >/dev/null 2>&1; then
        return 0
    fi
    raw="$(autospec_runtime_config_get "safety.issue_intent_gate.trusted_actors" "[]")"
    if ! printf '%s' "$raw" | jq -e . >/dev/null 2>&1; then
        return 0
    fi
    printf '%s' "$raw" | jq -r '.[]?.login // empty' 2>/dev/null || true
}

# control_channel_promote_label_actor ISSUE — print the login of the actor
# who applied the LAST `autospec:promote` labeled timeline event, or nothing
# on any gh/parse failure (callers MUST fail closed on empty). The command
# authority for promote is the LABEL, not just the issue author — anyone
# with triage rights can label an old trusted-authored issue, so the label
# applicator is verified too (mirrors approval_via_label in
# autonomous-provenance.sh, including the --paginate page-2 normalization:
# `gh api --paginate` emits one JSON array per page, flattened via
# `jq -es 'add'`).
control_channel_promote_label_actor() {
    local issue="$1" path out flat
    if [ -n "$REPO" ]; then
        path="repos/${REPO}/issues/${issue}/timeline"
    else
        # gh substitutes {owner}/{repo} from the current git remote.
        path="repos/{owner}/{repo}/issues/${issue}/timeline"
    fi
    out="$("$GH" api --paginate "$path" 2>/dev/null)" || return 0
    flat="$(printf '%s' "$out" | jq -es 'add // []' 2>/dev/null)" || return 0
    printf '%s' "$flat" | jq -r '
        [.[] | select(.event == "labeled" and .label.name == "autospec:promote")]
        | last | .actor.login // empty' 2>/dev/null || true
}

# control_channel_is_trusted_login LOGIN TRUSTED_NEWLINE_LIST — exit 0 only
# on an exact match (never test()/match() against issue-derived values, per
# feedback_jq_test_regex_metachar_injection).
control_channel_is_trusted_login() {
    local login="$1" trusted="$2" cand
    [ -n "$login" ] || return 1
    while IFS= read -r cand; do
        [ -n "$cand" ] || continue
        if [ "$cand" = "$login" ]; then
            return 0
        fi
    done <<EOF_TRUSTED
$trusted
EOF_TRUSTED
    return 1
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

        "autospec:promote")
            # Trusted-actor-only, fail closed on any missing/unparseable
            # data. TWO checks must both pass against
            # safety.issue_intent_gate.trusted_actors:
            #   1. the control issue's AUTHOR is a trusted login, AND
            #   2. the actor of the LAST `autospec:promote` labeled timeline
            #      event is a trusted login — the label is the command
            #      authority, and anyone with triage rights could label an
            #      old trusted-authored issue (approval-spoofing).
            local issue_number author_login label_actor trusted
            issue_number="$(printf '%s' "$issues_json" | jq -r '.[0].number // empty' 2>/dev/null || echo "")"
            author_login="$(printf '%s' "$issues_json" | jq -r '.[0].author.login // empty' 2>/dev/null || echo "")"

            if [ -z "$issue_number" ]; then
                warn "promote: could not extract issue number from JSON"
                return 0
            fi

            trusted="$(control_channel_trusted_logins)"
            label_actor="$(control_channel_promote_label_actor "$issue_number")"
            if control_channel_is_trusted_login "$author_login" "$trusted" \
                && control_channel_is_trusted_login "$label_actor" "$trusted"; then
                printf 'DECISION:promote\n'
                printf 'PROMOTE_ISSUE:%s\n' "$issue_number"
            else
                warn "promote: issue #$issue_number refused — author '$author_login' or label actor '$label_actor' is not a trusted actor (fail closed)"
                printf 'DECISION:promote-refused\n'
                printf 'PROMOTE_ISSUE:%s\n' "$issue_number"
            fi
            ;;

        "autospec:discard")
            local discard_issue_number
            discard_issue_number="$(printf '%s' "$issues_json" | jq -r '.[0].number // empty' 2>/dev/null || echo "")"

            if [ -z "$discard_issue_number" ]; then
                warn "discard: could not extract issue number from JSON"
                return 0
            fi

            printf 'DECISION:discard\n'
            printf 'DISCARD_ISSUE:%s\n' "$discard_issue_number"
            ;;

        "autospec:recalibrate-persona")
            # Write a flag file so the next conductor cycle triggers a persona
            # refresh / interview re-run.  Atomic via temp+mv; idempotent.
            local stamp
            stamp="$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'unknown')"
            mkdir -p "$FLAG_DIR"
            local _rtmp
            _rtmp="$(mktemp "${FLAG_DIR}/.persona-recalibrate.XXXXXX")"
            printf 'recalibrate\n%s\n' "$stamp" > "$_rtmp"
            mv "$_rtmp" "$RECALIBRATE_FLAG"
            printf 'DECISION:persona-recalibrate\n'
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Main: query each reserved label in severity order, emit decisions.
# ---------------------------------------------------------------------------
# Severity order: stop (highest) > pause > steer > priority > recalibrate-persona
# > promote > discard (lowest).
for label in $RESERVED_LABELS; do
    issues_json="$(query_label "$label")"
    emit_decision "$label" "$issues_json"
done

exit 0
