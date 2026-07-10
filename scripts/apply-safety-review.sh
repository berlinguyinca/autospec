#!/usr/bin/env bash
# scripts/apply-safety-review.sh — deterministic issue-intent safety stamper.
#
# Runs the safety linter (scripts/lint-issue-safety.sh) against an issue body,
# then either appends a passing `## Safety review` block and stamps
# `safety:reviewed` (--apply), or quarantines the issue with
# `security:quarantined` (--apply). Without --apply the script is report-only
# and mutates nothing. Fail-closed: an unparsable/indeterminate linter result
# is treated as non-PASS — never stamps safety:reviewed.
#
# Usage:
#   apply-safety-review.sh --issue N --repo OWNER/REPO --body-file <path> \
#       [--title TITLE] [--actor LOGIN] [--apply]
#
# Seams:
#   AUTOSPEC_LINT_ISSUE_SAFETY_BIN — linter binary (default: <script_dir>/lint-issue-safety.sh)
#   AUTOSPEC_GH_BIN                — gh binary (default: gh)
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LINT_BIN="${AUTOSPEC_LINT_ISSUE_SAFETY_BIN:-$SCRIPT_DIR/lint-issue-safety.sh}"
GH_BIN="${AUTOSPEC_GH_BIN:-gh}"

BEGIN_MARKER='<!-- autospec-safety:begin -->'
END_MARKER='<!-- autospec-safety:end -->'

ISSUE="" REPO="" BODY_FILE="" TITLE="" ACTOR="" APPLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --issue) ISSUE="${2:-}"; shift 2 ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        --body-file) BODY_FILE="${2:-}"; shift 2 ;;
        --title) TITLE="${2:-}"; shift 2 ;;
        --actor) ACTOR="${2:-}"; shift 2 ;;
        --apply) APPLY=1; shift ;;
        --help|-h)
            printf 'Usage: apply-safety-review.sh --issue N --repo OWNER/REPO --body-file <path> [--title TITLE] [--actor LOGIN] [--apply]\n'
            exit 0
            ;;
        *)
            printf 'apply-safety-review.sh: unknown option: %s\n' "$1" >&2
            exit 2
            ;;
    esac
done

[ -n "$ISSUE" ] || { printf 'apply-safety-review.sh: --issue required\n' >&2; exit 2; }
[ -n "$REPO" ] || { printf 'apply-safety-review.sh: --repo required\n' >&2; exit 2; }
[ -n "$BODY_FILE" ] || { printf 'apply-safety-review.sh: --body-file required\n' >&2; exit 2; }
[ -f "$BODY_FILE" ] || { printf 'apply-safety-review.sh: body file not found: %s\n' "$BODY_FILE" >&2; exit 2; }

# --- 1. Run the linter, capturing stdout + exit code independently. -------
set +e
lint_out="$("$LINT_BIN" --json --title "$TITLE" ${ACTOR:+--actor "$ACTOR"} "$BODY_FILE" 2>/dev/null)"
lint_rc=$?
set -e

decision="$(printf '%s' "$lint_out" | jq -r '.decision // empty' 2>/dev/null || printf '')"
findings_json="$(printf '%s' "$lint_out" | jq -c '.findings // []' 2>/dev/null || printf '')"
trusted_raw="$(printf '%s' "$lint_out" | jq -r '.trusted // empty' 2>/dev/null || printf '')"

if [ -z "$decision" ]; then
    # JSON unparsable — fall back to exit code.
    case "$lint_rc" in
        1) decision="SAFETY_AMBIGUOUS" ;;
        2) decision="SAFETY_BLOCK" ;;
        *) decision="" ;;
    esac
fi

# --- 2. Derive block fields. -----------------------------------------------
trust="untrusted"
if [ "$trusted_raw" = "true" ]; then
    trust="trusted"
fi

matched_rules="none"
if [ -n "$findings_json" ] && [ "$findings_json" != "[]" ] && [ "$findings_json" != "null" ]; then
    joined="$(printf '%s' "$findings_json" | jq -r '[.[].rule_id] | join(",")' 2>/dev/null || printf '')"
    if [ -n "$joined" ]; then
        matched_rules="$joined"
    fi
fi

reason="no blocking or ambiguous patterns matched"
if [ "$decision" != "SAFETY_PASS" ]; then
    reason="matched: $matched_rules"
fi

review_date="$(date -u +%F)"

# --- helpers ----------------------------------------------------------------

# Strip any existing "## Safety review" section (heading through end marker
# plus trailing metadata up to the next "## " heading or EOF) from a body
# file, writing the result to stdout. Ensures idempotent re-stamping.
strip_existing_block() {
    src_file="$1"
    python3 - "$src_file" "$BEGIN_MARKER" "$END_MARKER" <<'PY'
import re
import sys

path, begin, end = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(path, encoding="utf-8").read()

heading_re = re.compile(r"(?m)^## Safety review[ \t]*\r?\n")
m = heading_re.search(text)
if m is None or begin not in text or end not in text:
    sys.stdout.write(text)
    sys.exit(0)

end_idx = text.find(end)
if end_idx < 0:
    sys.stdout.write(text)
    sys.exit(0)
tail_start = end_idx + len(end)

# Find the next "## " heading after the end marker, or EOF.
next_heading = re.search(r"(?m)^## ", text[tail_start:])
if next_heading:
    tail_cut = tail_start + next_heading.start()
else:
    tail_cut = len(text)

new_text = text[: m.start()] + text[tail_cut:]
# Trim trailing whitespace left behind by the removed section.
new_text = new_text.rstrip("\n") + "\n"
sys.stdout.write(new_text)
PY
}

compose_block() {
    # $1 = decision literal to embed inside the markers
    block_decision="$1"
    # NOTE: no `- **actor:**` line. The autospec-run reader re-lints the whole
    # body with only the decision-marker block removed, so every line here must
    # be linter-safe. The issue author's raw login is user-controlled and can
    # contain linter substrings (e.g. a login like `liam` matches the `iam`
    # pattern), which would make the gate falsely reject its own PASS block. The
    # author is already recorded in issue metadata + the grooming audit comment,
    # so it is omitted here; trust/matched-rules/reason are generated, bounded,
    # and safe for a PASS.
    printf '\n## Safety review\n\n%s\n- **decision:** `%s`\n%s\n\n- **trust:** `%s`\n- **matched rules:** `%s`\n- **reason:** %s\n\n*Auto-reviewed by issue intent safety gate on %s.*\n' \
        "$BEGIN_MARKER" "$block_decision" "$END_MARKER" "$trust" "$matched_rules" "$reason" "$review_date"
}

write_new_body() {
    # $1 = decision literal for the block; writes composed body to stdout.
    stripped_file="$(mktemp "${TMPDIR:-/tmp}/apply-safety-review.stripped.XXXXXX")"
    strip_existing_block "$BODY_FILE" > "$stripped_file"
    cat "$stripped_file"
    compose_block "$1"
    rm -f "$stripped_file"
}

# --- 3/4. Branch on decision. ------------------------------------------------
if [ "$decision" = "SAFETY_PASS" ]; then
    new_body_file="$(mktemp "${TMPDIR:-/tmp}/apply-safety-review.body.XXXXXX")"
    write_new_body "SAFETY_PASS" > "$new_body_file"

    if [ "$APPLY" -eq 1 ]; then
        "$GH_BIN" label create safety:reviewed --force --repo "$REPO" >/dev/null 2>&1 || true
        "$GH_BIN" issue edit "$ISSUE" --repo "$REPO" --body-file "$new_body_file"
        "$GH_BIN" issue edit "$ISSUE" --repo "$REPO" --add-label safety:reviewed --remove-label security:quarantined
        rm -f "$new_body_file"
        jq -cn '{decision:"SAFETY_PASS",stamped:true}'
        exit 0
    fi

    rm -f "$new_body_file"
    jq -cn '{decision:"SAFETY_PASS",would_stamp:true}'
    exit 0
fi

# Non-PASS: AMBIGUOUS / BLOCK / indeterminate (fail-closed).
verdict="$decision"
if [ -z "$verdict" ]; then
    verdict="SAFETY_BLOCK"
fi

if [ "$APPLY" -eq 1 ]; then
    block_verdict="$verdict"
    case "$block_verdict" in
        SAFETY_AMBIGUOUS|SAFETY_BLOCK) : ;;
        *) block_verdict="SAFETY_BLOCK" ;;
    esac
    new_body_file="$(mktemp "${TMPDIR:-/tmp}/apply-safety-review.body.XXXXXX")"
    write_new_body "$block_verdict" > "$new_body_file"

    "$GH_BIN" label create security:quarantined --force --repo "$REPO" >/dev/null 2>&1 || true
    "$GH_BIN" issue edit "$ISSUE" --repo "$REPO" --body-file "$new_body_file"
    "$GH_BIN" issue edit "$ISSUE" --repo "$REPO" --add-label security:quarantined --remove-label auto-implement --remove-label needs-classify --remove-label safety:reviewed
    rm -f "$new_body_file"
    jq -cn --arg d "$verdict" '{decision:$d,stamped:false}'
    exit 1
fi

jq -cn --arg d "$verdict" '{decision:$d,would_stamp:false}'
exit 0
