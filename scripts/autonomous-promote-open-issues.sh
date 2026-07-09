#!/usr/bin/env bash
# scripts/autonomous-promote-open-issues.sh — Tier 1.5 real promotion command.
#
# Promotes genuinely-ready open issues into the `auto-implement` queue so the
# autonomous conductor's Tier 1.5 does real work instead of a no-op placeholder.
# The primary promotable class is `needs-classify` (listener-filed issues); each
# candidate is classified via scripts/classify-model-fit.sh (ctx:* / reasoning:*
# labels + a `## Model fit` body block), moved onto the `auto-implement` queue,
# and un-labeled `needs-classify`.
#
# SAFETY — DOUBLE-GATED. Defaults to REPORT-ONLY (dry). It mutates GitHub state
# ONLY when BOTH:
#   1. `--apply` is passed, AND
#   2. the env opt-in AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1 is set.
# Without both it computes what it WOULD promote and lists those candidates in
# `skipped` with reason "report-only" — it never calls `gh issue edit`. This is
# the blast-radius guard: the conductor auto-detects this script by path, so
# default-safe behavior keeps a live conductor from auto-labeling on merge.
#
# CONSERVATIVE SELECTOR — a candidate must meet ALL of:
#   - open, and NOT already labeled `auto-implement`;
#   - labeled `needs-classify` (the primary promotable class);
#   - NOT labeled any of: epic, needs-autospec-template, blocked* (any),
#     paused-by-user, locked-by-autospec-processor, autospec:needs-human,
#     wontfix, duplicate;
#   - non-trivial body (>= 80 chars).
# When in doubt, SKIP (recorded in `skipped` with a reason). Under-promotion is
# far safer than feeding ill-formed work to an admin-auto-merge loop.
#
# Usage:
#   autonomous-promote-open-issues.sh [--repo OWNER/REPO] [--apply]
#   autonomous-promote-open-issues.sh --help
#
# Output: a single JSON object on stdout:
#   {"dry":<bool>,"filed":<int>,"promoted":[<#>...],
#    "skipped":[{"issue":<#>,"reason":"..."}],"reason":"..."}
#
# Exit codes:
#   0  — success (report-only or apply)
#   2  — usage error

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

MIN_BODY_CHARS="${AUTOSPEC_PROMOTE_MIN_BODY_CHARS:-80}"

usage() {
    cat <<'EOF'
autonomous-promote-open-issues.sh — Tier 1.5 promote-open-issues command

Promotes ready open `needs-classify` issues into the `auto-implement` queue.
Double-gated: mutates GitHub state only with --apply AND
AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1. Otherwise report-only (dry).

Usage:
  autonomous-promote-open-issues.sh [--repo OWNER/REPO] [--apply]
  autonomous-promote-open-issues.sh --help

Emits one JSON object:
  {"dry":<bool>,"filed":<int>,"promoted":[...],"skipped":[...],"reason":"..."}
EOF
}

die() {
    printf 'autonomous-promote-open-issues: %s\n' "$1" >&2
    exit 2
}

repo=""
apply_flag=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --apply) apply_flag=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

# ── Double gate: apply mutates only with flag AND env opt-in ──────────────────
apply_enabled=0
if [ "$apply_flag" = "1" ] && [ "${AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY:-}" = "1" ]; then
    apply_enabled=1
fi

# ── Resolve repo ──────────────────────────────────────────────────────────────
if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

# Accumulators.
promoted_nums=""          # space-separated issue numbers actually promoted
SKIPPED_FILE="$(mktemp -t promote-skipped.XXXXXX)"
trap 'rm -f "$SKIPPED_FILE"' EXIT
printf '[]\n' > "$SKIPPED_FILE"

record_skip() {
    # $1 = issue number, $2 = reason
    jq -c --argjson issue "$1" --arg reason "$2" '. + [{issue:$issue, reason:$reason}]' \
        "$SKIPPED_FILE" > "$SKIPPED_FILE.tmp" && mv "$SKIPPED_FILE.tmp" "$SKIPPED_FILE"
}

emit_json() {
    # $1 = dry (true|false), $2 = reason
    local dry="$1" reason="$2"
    local filed=0
    local promoted_json="[]"
    if [ -n "$promoted_nums" ]; then
        promoted_json="$(printf '%s\n' $promoted_nums | jq -R 'tonumber' | jq -s .)"
        filed="$(printf '%s\n' $promoted_nums | awk 'NF' | wc -l | tr -d ' ')"
    fi
    jq -n \
        --argjson dry "$dry" \
        --argjson filed "$filed" \
        --argjson promoted "$promoted_json" \
        --slurpfile skipped "$SKIPPED_FILE" \
        --arg reason "$reason" \
        '{dry:$dry, filed:$filed, promoted:$promoted, skipped:$skipped[0], reason:$reason}'
}

# ── Fetch candidate set: open needs-classify issues ───────────────────────────
issues_json="$(gh issue list \
    --repo "$repo" \
    --state open \
    --label needs-classify \
    --limit 200 \
    --json number,title,labels,body 2>/dev/null || printf '[]')"
[ -n "$issues_json" ] || issues_json='[]'

candidate_count="$(printf '%s' "$issues_json" | jq 'length' 2>/dev/null || echo 0)"

# Labels that disqualify a candidate outright (exact match). `blocked*` is
# handled separately as a prefix match below.
is_excluded_label() {
    case "$1" in
        epic|needs-autospec-template|paused-by-user|locked-by-autospec-processor|autospec:needs-human|wontfix|duplicate)
            return 0 ;;
        blocked*)
            return 0 ;;
        *)
            return 1 ;;
    esac
}

numbers="$(printf '%s' "$issues_json" | jq -r '.[].number' 2>/dev/null || true)"

for num in $numbers; do
    issue="$(printf '%s' "$issues_json" | jq -c --argjson n "$num" '.[] | select(.number == $n)')"
    body="$(printf '%s' "$issue" | jq -r '.body // ""')"
    labels="$(printf '%s' "$issue" | jq -r '.labels[]?.name // empty')"

    # Already queued for implementation → skip.
    if printf '%s\n' "$labels" | grep -qx 'auto-implement'; then
        record_skip "$num" "already-auto-implement"
        continue
    fi

    # Disqualifying labels (exact set + blocked* prefix).
    excluded_reason=""
    while IFS= read -r label; do
        [ -n "$label" ] || continue
        if is_excluded_label "$label"; then
            excluded_reason="excluded-label:$label"
            break
        fi
    done <<EOF
$labels
EOF
    if [ -n "$excluded_reason" ]; then
        record_skip "$num" "$excluded_reason"
        continue
    fi

    # Non-trivial body gate.
    body_len="$(printf '%s' "$body" | wc -c | tr -d ' ')"
    if [ "${body_len:-0}" -lt "$MIN_BODY_CHARS" ]; then
        record_skip "$num" "insufficient-body"
        continue
    fi

    # ── This issue is a genuine promotion candidate. ─────────────────────────
    if [ "$apply_enabled" != "1" ]; then
        # Report-only: list what we WOULD promote, mutate nothing.
        record_skip "$num" "report-only"
        continue
    fi

    # ── Apply mode: classify + promote. ──────────────────────────────────────
    body_file="$(mktemp -t promote-body.XXXXXX)"
    printf '%s' "$body" > "$body_file"

    classify_json="$(bash "$SCRIPT_DIR/classify-model-fit.sh" "$body_file" --json 2>/dev/null || printf '{}')"
    ctx="$(printf '%s' "$classify_json" | jq -r '.ctx // "64k"' 2>/dev/null || echo '64k')"
    reasoning="$(printf '%s' "$classify_json" | jq -r '.reasoning // "medium"' 2>/dev/null || echo 'medium')"
    [ -n "$ctx" ] || ctx="64k"
    [ -n "$reasoning" ] || reasoning="medium"

    # Ensure labels exist (idempotent). Mirror autospec-classify colors.
    gh label create "auto-implement" --color "0e8a16" --repo "$repo" --force >/dev/null 2>&1 || true
    gh label create "ctx:${ctx}" --color "c5def5" --repo "$repo" --force >/dev/null 2>&1 || true
    gh label create "reasoning:${reasoning}" --color "c2e0c6" --repo "$repo" --force >/dev/null 2>&1 || true

    # Move onto the implementation queue: add auto-implement + fit labels,
    # remove needs-classify.
    gh issue edit "$num" --repo "$repo" \
        --add-label "auto-implement,ctx:${ctx},reasoning:${reasoning}" \
        --remove-label "needs-classify" >/dev/null 2>&1 || true

    # Append the `## Model fit` block (reuse classify-model-fit.sh non-json
    # form so the block markers/format match the classifier exactly). Idempotent:
    # skip if the block is already present.
    if ! printf '%s' "$body" | grep -q 'autospec-classify:begin'; then
        fit_block="$(bash "$SCRIPT_DIR/classify-model-fit.sh" "$body_file" 2>/dev/null || printf '')"
        if [ -n "$fit_block" ]; then
            new_body_file="$(mktemp -t promote-newbody.XXXXXX)"
            {
                printf '%s\n\n' "$body"
                printf '%s\n' "$fit_block"
            } > "$new_body_file"
            gh issue edit "$num" --repo "$repo" --body-file "$new_body_file" >/dev/null 2>&1 || true
            rm -f "$new_body_file"
        fi
    fi

    rm -f "$body_file"
    promoted_nums="${promoted_nums}${promoted_nums:+ }${num}"
done

# ── Emit result ───────────────────────────────────────────────────────────────
if [ "$apply_enabled" = "1" ]; then
    if [ -n "$promoted_nums" ]; then
        emit_json "false" "promoted ready needs-classify issues"
    else
        emit_json "true" "no-promotable-issues"
    fi
else
    if [ "${candidate_count:-0}" -gt 0 ]; then
        emit_json "true" "report-only (set --apply and AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1 to promote)"
    else
        emit_json "true" "no open needs-classify issues"
    fi
fi
