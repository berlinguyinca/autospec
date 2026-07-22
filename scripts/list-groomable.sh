#!/usr/bin/env bash
# scripts/list-groomable.sh — deterministic candidate selection for backlog grooming.
#
# Lists open issues that are groomable: not excluded by a disqualifying label,
# not a duplicate of a closed no-op finding (title+body hash match), sorted
# oldest-first (ascending issue number) and capped at --budget.
#
# Usage:
#   list-groomable.sh --repo OWNER/REPO --budget N
#
# Output: a single JSON object on stdout:
#   {"candidates":[{"number":N,"title":"...","class":"needs-classify|needs-template|unlabeled"}],
#    "skipped":[{"number":N,"reason":"..."}]}
#
# Fail-closed: empty or malformed `gh` output yields
#   {"candidates":[],"skipped":[]}
#
# Exit codes:
#   0 — success
#   2 — usage error

set -eu

die() {
    printf 'list-groomable: %s\n' "$1" >&2
    exit 2
}

repo=""
budget=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --budget) budget="${2:-}"; shift 2 ;;
        --help|-h)
            cat <<'EOF'
list-groomable.sh — deterministic candidate selection + closed-no-op dedup

Usage:
  list-groomable.sh --repo OWNER/REPO --budget N

Emits one JSON object:
  {"candidates":[{"number":N,"title":"...","class":"..."}],"skipped":[{"number":N,"reason":"..."}]}
EOF
            exit 0
            ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$repo" ] || die "--repo is required"
[ -n "$budget" ] || die "--budget is required"

EMPTY_RESULT='{"candidates":[],"skipped":[]}'

# ── Fetch open issues ───────────────────────────────────────────────────────
open_json="$(gh issue list \
    --repo "$repo" \
    --state open \
    --json number,title,labels,body \
    --limit 200 2>/dev/null || printf '')"

if [ -z "$open_json" ]; then
    printf '%s\n' "$EMPTY_RESULT"
    exit 0
fi

if ! printf '%s' "$open_json" | jq -e 'type == "array"' >/dev/null 2>&1; then
    printf '%s\n' "$EMPTY_RESULT"
    exit 0
fi

# ── Fetch closed issues (once) for no-op dedup ──────────────────────────────
closed_json="$(gh issue list \
    --repo "$repo" \
    --state closed \
    --json number,title,body \
    --limit 200 2>/dev/null || printf '[]')"
if [ -z "$closed_json" ]; then
    closed_json='[]'
fi
if ! printf '%s' "$closed_json" | jq -e 'type == "array"' >/dev/null 2>&1; then
    closed_json='[]'
fi

# ── Build closed-issue hash set (sha256 of title + first-200-body-chars) ───
# Declared up front so the single EXIT trap can clean them even if `set -e` fires
# between their mktemp below and the tail cleanup (leak-on-early-exit guard).
dedup_skips_file=""
kept_file=""
closed_hashes_file="$(mktemp -t list-groomable-closed.XXXXXX)"
trap 'rm -f "$closed_hashes_file" "$dedup_skips_file" "$kept_file"' EXIT

# Compute the dedup hash for a single issue JSON object. Basis is
# title + first-200-codepoints-of-body, built entirely in jq (`-j` = raw, no
# trailing newline) and piped straight into shasum. This is used verbatim on
# BOTH the closed side and the open side so the hash basis is symmetric:
#   - embedded newlines in title/body are preserved (no `while read` splitting),
#   - truncation is jq codepoint `[0:200]` on both sides (no cut/byte skew),
#   - no command substitution, so no trailing-newline stripping asymmetry.
compute_hash() {
    # $1 = single issue JSON object
    printf '%s' "$1" | jq -j '(.title // "") + ((.body // "")[0:200])' | shasum -a 256 | awk '{print $1}'
}

# Build the closed-issue hash set, one record at a time (jq -c emits one compact
# object per line; embedded newlines are JSON-escaped, so line-splitting is safe
# here — the raw text is only ever reconstructed inside compute_hash via jq).
printf '%s' "$closed_json" | jq -c '.[]' | \
while IFS= read -r closed_issue; do
    [ -n "$closed_issue" ] || continue
    compute_hash "$closed_issue"
done > "$closed_hashes_file"

# ── Classify + filter open issues via jq ────────────────────────────────────
excluded_labels='["auto-implement","no-auto","epic","paused-by-user","autospec:needs-human","wontfix","duplicate","security:quarantined"]'

filtered_json="$(printf '%s' "$open_json" | jq -c --argjson excluded "$excluded_labels" '
    def is_excluded_label:
        . as $l
        | ($excluded | index($l)) != null
        or ($l | startswith("hold:"))
        or ($l | startswith("locked-"));

    map(
        . as $issue
        | ($issue.labels // [] | map(.name)) as $labelnames
        | {
            number: $issue.number,
            title: ($issue.title // ""),
            body: ($issue.body // ""),
            excluded: (($labelnames | map(select(is_excluded_label)) | length) > 0),
            class: (
                if ($labelnames | index("needs-autospec-template")) != null then "needs-template"
                elif ($labelnames | index("needs-classify")) != null then "needs-classify"
                else "unlabeled"
                end
            )
        }
    )
' 2>/dev/null || printf '[]')"

if [ -z "$filtered_json" ]; then
    filtered_json='[]'
fi

# ── Apply exclusion, then dedup against closed no-op hashes, then sort+cap ──
# jq alone cannot compute sha256, so hash matching is done in the shell loop below.
candidates_pre="$(printf '%s' "$filtered_json" | jq -c '[.[] | select(.excluded | not)]' 2>/dev/null || printf '[]')"
excluded_skips="$(printf '%s' "$filtered_json" | jq -c '[.[] | select(.excluded) | {number: .number, reason: "excluded-label"}]' 2>/dev/null || printf '[]')"

dedup_skips_file="$(mktemp -t list-groomable-dedup.XXXXXX)"
printf '[]' > "$dedup_skips_file"
kept_file="$(mktemp -t list-groomable-kept.XXXXXX)"
printf '[]' > "$kept_file"

count="$(printf '%s' "$candidates_pre" | jq 'length')"
i=0
while [ "$i" -lt "$count" ]; do
    issue="$(printf '%s' "$candidates_pre" | jq -c ".[$i]")"
    hash="$(compute_hash "$issue")"
    number="$(printf '%s' "$issue" | jq -r '.number')"

    if grep -qx "$hash" "$closed_hashes_file" 2>/dev/null; then
        jq -c --argjson n "$number" '. + [{number: $n, reason: "dup-of-closed-noop"}]' \
            "$dedup_skips_file" > "$dedup_skips_file.tmp" && mv "$dedup_skips_file.tmp" "$dedup_skips_file"
    else
        jq -c --argjson issue "$issue" '. + [$issue]' \
            "$kept_file" > "$kept_file.tmp" && mv "$kept_file.tmp" "$kept_file"
    fi
    i=$((i + 1))
done

final_candidates="$(jq -c --argjson budget "$budget" \
    '[.[] | {number, title, class}] | sort_by(.number) | .[0:$budget]' \
    "$kept_file")"

final_skipped="$(jq -c -n --slurpfile a <(printf '%s' "$excluded_skips") --slurpfile b "$dedup_skips_file" \
    '($a[0] // []) + ($b[0] // [])')"

jq -n --argjson candidates "$final_candidates" --argjson skipped "$final_skipped" \
    '{candidates: $candidates, skipped: $skipped}'

rm -f "$dedup_skips_file" "$kept_file"
