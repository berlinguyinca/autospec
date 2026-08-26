#!/usr/bin/env bash
# scripts/project-board-control-mirror.sh — mirror project-level Tier-0
# control labels from one designated control issue into each fleet repo.
#
# WHY: Tier 0 (autonomous-control-channel.sh) reads reserved control labels
# per repo. A board-driven fleet spans many repos, so a project-level
# "stop everything" needs to reach every worker. This script reads the four
# reserved labels off ONE control issue and mirrors them into each repo in
# the fleet, so each repo's own Tier-0 poll (`gh issue list --label X
# --state open`) picks them up locally — no cross-repo issue-number coupling
# required.
#
# SAFETY PROPERTIES (all three are load-bearing — do not "simplify" them):
#
#   1. ADDITIVE ONLY. A label is applied to a repo that lacks it and is
#      NEVER removed. A repo that paused itself locally must never be
#      un-paused just because the board doesn't carry that label. This
#      script never invokes --remove-label.
#
#   2. Opt-in by configuration. When --control-issue is unset, project-level
#      control is DISABLED entirely: prints an empty envelope and exits 0.
#      A board must never gain implicit authority over a repo that didn't
#      opt in.
#
#   3. The control issue itself must be inside --allowlist, or mirroring is
#      disabled wholesale (code_health:project_board_repo_out_of_scope,
#      nothing written). Each target repo is independently checked against
#      the same allowlist; a repo outside it is skipped and no `gh` call
#      may ever name it.
#
# MIRROR TARGET (the open design question in the brief):
#
#   The brief's sketch called `gh issue edit "$ctl_num" --repo "$repo"
#   --add-label ...` — i.e. reusing the CONTROL issue's number verbatim as
#   the issue number to edit in every target repo. That number is
#   essentially random with respect to the target repo: it may not exist,
#   or worse, may already be someone else's unrelated issue, and this
#   script would then slap `autospec:stop` on it. That is a live
#   mislabeling hazard given `set -eu` treats a missing/wrong issue as
#   "worked" as long as gh exits 0.
#
#   Instead, per target repo, this script finds-or-creates ONE dedicated
#   marker issue that this script itself owns (identified by a fixed,
#   constant title — never derived from board/user input), and mirrors
#   labels onto THAT issue only. Every target repo's own Tier-0
#   (`gh issue list --label <label> --state open`) does not care which
#   issue carries the label, so a dedicated marker issue is functionally
#   equivalent to editing "the right" issue, while being categorically
#   safe: this script can only ever mutate an issue it created itself,
#   never an arbitrary pre-existing one.
#
#   The marker is FOUND by filtering on a dedicated, non-reserved label
#   ($MARKER_LABEL, always applied alongside the mirrored labels at create
#   time) — not by `gh issue list --search ... in:title`. GitHub's search
#   index lags real-time issue state by seconds to minutes, so a
#   just-created marker can look invisible to a text search on the very
#   next cycle; a plain `--label` filter is a direct, non-search query and
#   does not have that lag. This removes the search-lag hazard at the
#   source rather than merely tolerating it.
#
#   FIND-FAILURE HANDLING: a marker lookup that errors (non-zero gh exit)
#   or returns something that is not a JSON array (malformed payload, an
#   API error object, a rate-limit message that still happens to be valid
#   JSON) is "could not determine", NOT "found nothing". Only a
#   successful lookup that parses to an actual (possibly empty) JSON array
#   is trustworthy enough to decide create-vs-edit. On "could not
#   determine" the repo is skipped for this cycle with a reason — creating
#   on uncertainty would pile up duplicate marker issues in the operator's
#   repo over a multi-week unattended run, which is worse than a missed
#   mirror cycle (recoverable on the next poll).
#
#   MULTIPLE MATCHES: if more than one open issue carries $MARKER_LABEL
#   (e.g. an operator manually relabeled something, or a prior bug created
#   a duplicate), this script picks the one with the LOWEST issue number
#   — the oldest, i.e. the first marker this script ever created for that
#   repo — deterministically, via `sort_by(.number)`, never the API's
#   default response order (which is not guaranteed stable). It edits that
#   one and never creates a new one when at least one match already
#   exists.
#
# Usage:
#   project-board-control-mirror.sh [--control-issue owner/repo#N] \
#       --repos a,b --allowlist 'pat,pat'
#
# Output (stdout): single JSON object:
#   {"mirrored":[{"repo","label"}],"skipped":[{"repo","reason"}]}
#
# Exit codes: always 0 on recognized usage (fail-open by design — this is a
# read-mostly relay, never allowed to abort a caller's loop). 2 on a
# genuinely malformed CLI invocation (unknown flag).
#
# Bash safety rules:
#   - set -eu; no RETURN traps (they leak under set -u)
#   - if/then/fi for all one-sided conditionals (not `[ x ] && action`)
#   - allowlist/repo matching is LITERAL prefix/equality only — config- and
#     board-derived strings are never treated as regex (see allowed())
#   - jq: never interpolate host/board-derived values into test()/match();
#     use --arg + index()/== for equality matching
#   - no eval, ever
#
# Reserved-label matching is intentionally CASE-SENSITIVE (plain string
# equality against a fixed literal set) — "Autospec:Stop" or
# "autospec:STOP" is not one of the four reserved labels and must never be
# mirrored. Do not "helpfully" make this case-insensitive; that would
# widen the set of board labels that carry control authority.

set -eu

RESERVED="autospec:stop autospec:pause autospec:priority autospec:steer"
MARKER_TITLE="[autospec] project-board control relay (do not edit manually)"
MARKER_BODY="This issue is a machine-managed relay target for project-board-control-mirror.sh. It exists so Tier-0 control labels mirrored from the fleet's designated control issue land on an issue this script owns, never on an arbitrary pre-existing one. Do not close or repurpose it."
MARKER_LABEL="autospec:project-board-marker"

GH="${AUTOSPEC_GH_CMD:-gh}"

control=""; repos=""; allowlist=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --control-issue) control="${2:-}";   shift 2 ;;
        --repos)         repos="${2:-}";     shift 2 ;;
        --allowlist)     allowlist="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'project-board-control-mirror.sh [--control-issue o/r#N] --repos a,b --allowlist pat\n'
            exit 0
            ;;
        *)
            printf 'project-board-control-mirror: unknown option: %s\n' "$1" >&2
            exit 2
            ;;
    esac
done

# ---------------------------------------------------------------------------
# allowed REPO — literal prefix/equality match against --allowlist, never a
# regex. A pattern ending in '*' matches by literal prefix (the '*' is the
# only wildcard, and only as a trailing glob); any other pattern must match
# exactly. Quoting the pattern portion inside the case statement disables
# glob interpretation of the pattern's own contents (only the unquoted
# trailing '*' remains a wildcard), so metacharacters like '.', '(', '|',
# '[' in either the pattern or the repo name are compared literally.
# ---------------------------------------------------------------------------
allowed() {
    _r="$1"
    _old_ifs="$IFS"
    IFS=','
    for _pat in $allowlist; do
        case "$_pat" in
            *\*)
                case "$_r" in
                    "${_pat%\*}"*) IFS="$_old_ifs"; return 0 ;;
                esac
                ;;
            *)
                if [ "$_r" = "$_pat" ]; then
                    IFS="$_old_ifs"
                    return 0
                fi
                ;;
        esac
    done
    IFS="$_old_ifs"
    return 1
}

emit_empty() {
    printf '{"mirrored":[],"skipped":[]}\n'
}

# No control issue configured: project-level control is disabled entirely.
if [ -z "$control" ]; then
    emit_empty
    exit 0
fi

ctl_repo="${control%%#*}"
ctl_num="${control##*#}"

# The control issue itself must be in scope, or mirroring is disabled
# wholesale and NOTHING is written — not even a gh call naming ctl_repo.
if ! allowed "$ctl_repo"; then
    printf 'code_health:project_board_repo_out_of_scope control_issue=%s\n' "$control" >&2
    emit_empty
    exit 0
fi

labels_json="$("$GH" issue view "$ctl_num" --repo "$ctl_repo" --json labels 2>/dev/null || printf '[]')"
if ! printf '%s' "$labels_json" | jq -e . >/dev/null 2>&1; then
    labels_json='[]'
fi

# labels_to_mirror: the subset of RESERVED present on the control issue.
# Only these four literal label names are ever considered — any other
# label on the control issue (bug, documentation, ...) is ignored, so board
# write access can never get an arbitrary label mirrored across the fleet.
labels_to_mirror=""
for label in $RESERVED; do
    present="$(printf '%s' "$labels_json" | jq -r --arg l "$label" '
        (if type == "array" then . else (.labels // []) end)
        | map(.name) | index($l) // empty
    ' 2>/dev/null || printf '')"
    if [ -n "$present" ]; then
        labels_to_mirror="$labels_to_mirror $label"
    fi
done

mirrored='[]'
skipped='[]'

_old_ifs="$IFS"
IFS=','
for repo in $repos; do
    IFS="$_old_ifs"

    if [ -z "$repo" ]; then
        IFS=','
        continue
    fi

    if ! allowed "$repo"; then
        skipped="$(printf '%s' "$skipped" | jq --arg r "$repo" --arg reason "repo_out_of_scope" '. + [{repo:$r,reason:$reason}]')"
        IFS=','
        continue
    fi

    if [ -z "$labels_to_mirror" ]; then
        IFS=','
        continue
    fi

    # Find this script's own marker issue in the target repo via a plain
    # label filter (not full-text --search — see FIND-FAILURE HANDLING in
    # the header). Capture success/failure explicitly: an `if cmd; then`
    # guard does not trip `set -e`, unlike a bare `cmd || fallback` on an
    # assignment, and we need the failure signal, not just a fallback value.
    marker_raw=""
    marker_lookup_ok=0
    if marker_raw="$("$GH" issue list --repo "$repo" --label "$MARKER_LABEL" --state open --json number 2>/dev/null)"; then
        marker_lookup_ok=1
    fi

    marker_valid=0
    if [ "$marker_lookup_ok" -eq 1 ]; then
        if printf '%s' "$marker_raw" | jq -e 'type == "array"' >/dev/null 2>&1; then
            marker_valid=1
        fi
    fi

    # "Could not determine" (gh failed, or the payload isn't a JSON array —
    # malformed output, a rate-limit/error object that still parses as
    # JSON) is NEVER treated as "found nothing". Skip this repo for this
    # cycle instead of risking a duplicate marker issue.
    if [ "$marker_valid" -ne 1 ]; then
        skipped="$(printf '%s' "$skipped" | jq --arg r "$repo" --arg reason "marker_lookup_failed" '. + [{repo:$r,reason:$reason}]')"
        IFS=','
        continue
    fi

    # Deterministic pick on multiple matches: the lowest issue number (the
    # oldest, i.e. the first marker this script ever created for this
    # repo) via sort_by — never the API's default response order.
    marker_num="$(printf '%s' "$marker_raw" | jq -r 'sort_by(.number) | .[0].number // empty' 2>/dev/null || printf '')"

    if [ -n "$marker_num" ]; then
        for label in $labels_to_mirror; do
            "$GH" issue edit "$marker_num" --repo "$repo" --add-label "$label" >/dev/null 2>&1 || true
            mirrored="$(printf '%s' "$mirrored" | jq --arg r "$repo" --arg l "$label" '. + [{repo:$r,label:$l}]')"
        done
    else
        # Confirmed-empty lookup: safe to create. The marker label is
        # attached at creation time so the NEXT cycle's --label filter
        # finds it immediately (no search-index lag to race).
        set -- "$GH" issue create --repo "$repo" --title "$MARKER_TITLE" --body "$MARKER_BODY" --label "$MARKER_LABEL"
        for label in $labels_to_mirror; do
            set -- "$@" --label "$label"
        done
        "$@" >/dev/null 2>&1 || true
        for label in $labels_to_mirror; do
            mirrored="$(printf '%s' "$mirrored" | jq --arg r "$repo" --arg l "$label" '. + [{repo:$r,label:$l}]')"
        done
    fi

    IFS=','
done
IFS="$_old_ifs"

printf '{"mirrored":%s,"skipped":%s}\n' "$mirrored" "$skipped"
