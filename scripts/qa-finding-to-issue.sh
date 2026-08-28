#!/usr/bin/env bash
# scripts/qa-finding-to-issue.sh — translate a qa-verdict.json finding
# into an auto-implement,autospec:v2-flow GitHub issue (issue #663).
#
# Args:
#   --finding '<json>'    JSON object from .findings[] of qa-verdict.json
#   --dry-run             Print the issue body but do not call gh.
#   --dedup-cache PATH    Use PATH as the dedup cache (default
#                         .autospec/heal-dedup.txt). One line per
#                         (category, file, summary_hash) tuple.
#
# Exit codes:
#   0 — issue filed successfully
#   1 — dedup-skip (an equivalent open issue already exists)
#   2 — error (malformed JSON / gh call failure / missing args)

set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
project_sync_issue() {
    local helper="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR/../skills/autospec-shared/scripts}/project-sync-issue.sh"
    bash "$helper" "$1" "${REPO_DIR:-$PWD}"
}

FINDING=""
DRY_RUN=0
DEDUP_CACHE=".autospec/heal-dedup.txt"

while [ $# -gt 0 ]; do
    case "$1" in
        --finding)     FINDING="$2"; shift ;;
        --dry-run)     DRY_RUN=1 ;;
        --dedup-cache) DEDUP_CACHE="$2"; shift ;;
        --help|-h)
            sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *) echo "qa-finding-to-issue: unknown arg: $1" >&2; exit 2 ;;
    esac
    shift
done

if [ -z "$FINDING" ]; then
    echo "qa-finding-to-issue: --finding required" >&2
    exit 2
fi

# Validate JSON.
echo "$FINDING" | jq . >/dev/null 2>&1 || {
    echo "qa-finding-to-issue: --finding is not valid JSON" >&2
    exit 2
}

CATEGORY=$(echo "$FINDING" | jq -r '.category // "other"')
SUMMARY=$(echo "$FINDING" | jq -r '.summary // ""')
EVIDENCE=$(echo "$FINDING" | jq -r '.evidence // ""')
FILE=$(echo "$FINDING" | jq -r '.file // ""')
STATUS=$(echo "$FINDING" | jq -r '.status // "PARTIAL"')
REMEDIATION=$(echo "$FINDING" | jq -r '.remediation // ""')

# Dedup key: (category, file, summary_hash).
SUMMARY_HASH=$(printf '%s' "$SUMMARY" | shasum -a 256 | awk '{print substr($1,1,12)}')
DEDUP_KEY="${CATEGORY}|${FILE}|${SUMMARY_HASH}"

mkdir -p "$(dirname "$DEDUP_CACHE")" 2>/dev/null || true
if [ -f "$DEDUP_CACHE" ] && grep -Fxq "$DEDUP_KEY" "$DEDUP_CACHE"; then
    echo "qa-finding-to-issue: dedup-skip ($DEDUP_KEY)" >&2
    exit 1
fi

# Also check open issues with the same title prefix (best-effort; needs gh
# in real mode but skipped under --dry-run).
TITLE="fix: ${CATEGORY} — ${SUMMARY}"
TITLE=$(printf '%s' "$TITLE" | head -c 200)

if [ "$DRY_RUN" != 1 ] && command -v gh >/dev/null 2>&1; then
    if gh issue list --state open --search "$TITLE in:title" --json number --jq 'length' 2>/dev/null | grep -q '^[1-9]'; then
        echo "$DEDUP_KEY" >> "$DEDUP_CACHE"
        echo "qa-finding-to-issue: dedup-skip (existing open issue)" >&2
        exit 1
    fi
fi

REMEDIATION_FALLBACK="Apply the code-health directive for category ${CATEGORY} from AGENTS.md."
EVIDENCE_TEXT="${EVIDENCE:-_no evidence reference recorded_}"
REMEDIATION_TEXT="${REMEDIATION:-$REMEDIATION_FALLBACK}"
BODY=$(printf '%s\n' \
"## Category" \
"" \
"${CATEGORY}" \
"" \
"## Summary" \
"" \
"${SUMMARY}" \
"" \
"## Status" \
"" \
"${STATUS}" \
"" \
"## Evidence" \
"" \
"${EVIDENCE_TEXT}" \
"" \
"## Reproduction" \
"" \
"Run /autospec-qa against the current HEAD. The finding above must" \
"re-emit via the verify-first probe before this issue is acted on." \
"" \
"## Remediation directive" \
"" \
"${REMEDIATION_TEXT}" \
"" \
"---" \
"" \
"Filed automatically by scripts/qa-finding-to-issue.sh from a" \
"release-blocking /autospec-qa finding (issue #663 self-healing loop).")

if [ "$DRY_RUN" = 1 ]; then
    printf 'TITLE: %s\n\n%s\n' "$TITLE" "$BODY"
    echo "$DEDUP_KEY" >> "$DEDUP_CACHE"
    exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
    echo "qa-finding-to-issue: gh not installed; cannot file" >&2
    exit 2
fi

# origin:self provenance (issue #1744): idempotent, best-effort label
ensure_origin_self_label() {
    gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
}

ensure_origin_self_label

if ISSUE_URL="$(gh issue create \
    --title "$TITLE" \
    --label "auto-implement,autospec:v2-flow" \
    --label origin:self \
    --body "$BODY" 2>/dev/null)"; then
    project_sync_issue "$ISSUE_URL" || exit $?
    echo "$DEDUP_KEY" >> "$DEDUP_CACHE"
    exit 0
fi
echo "qa-finding-to-issue: gh issue create failed" >&2
exit 2
