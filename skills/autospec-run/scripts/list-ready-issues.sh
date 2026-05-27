#!/usr/bin/env bash
# list-ready-issues.sh — compute safe autospec-run distributed queue candidates.

set -eu

usage() {
    cat <<'EOF'
Usage: list-ready-issues.sh [--repo owner/repo] [--batch-size N]

Outputs JSON with ready, blocked, claimed, conflicts, and batch arrays.
EOF
}

die() {
    printf 'list-ready-issues: %s\n' "$1" >&2
    exit 1
}

repo=""
batch_size=3

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --batch-size) batch_size="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$batch_size" in *[!0-9]*|'') die "--batch-size must be an integer" ;; esac
[ "$batch_size" -gt 0 ] || batch_size=3

if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

issue_list() {
    label="$1"
    gh issue list \
        --repo "$repo" \
        --state open \
        --label "$label" \
        --limit 200 \
        --json number,title,body,labels
}

AUTO_FILE="$(mktemp -t autospec-auto-issues.XXXXXX)"
ACTIVE_FILE="$(mktemp -t autospec-active-issues.XXXXXX)"
READY_FILE="$(mktemp -t autospec-ready.XXXXXX)"
BLOCKED_FILE="$(mktemp -t autospec-blocked.XXXXXX)"
CONFLICTS_FILE="$(mktemp -t autospec-conflicts.XXXXXX)"
trap 'rm -f "$AUTO_FILE" "$ACTIVE_FILE" "$READY_FILE" "$BLOCKED_FILE" "$CONFLICTS_FILE"' EXIT

issue_list auto-implement > "$AUTO_FILE"
issue_list in-progress-by-bot > "$ACTIVE_FILE"
printf '[]\n' > "$READY_FILE"
printf '[]\n' > "$BLOCKED_FILE"
printf '[]\n' > "$CONFLICTS_FILE"

extract_paths() {
    jq -r '.body // ""' | awk '
      /^## Implementation outline[[:space:]]*$/ { in_outline=1; next }
      /^## / && in_outline { exit }
      in_outline { print }
    ' | grep -Eo '`[^`]+`' | tr -d '`' | grep -E '[/][^[:space:]]+|^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+' || true
}

extract_deps() {
    jq -r '.body // ""' | grep -Eo 'Depends on (#|issue )[0-9]+' | grep -Eo '[0-9]+' || true
}

issue_state() {
    gh issue view "$1" --repo "$repo" --json state --jq .state 2>/dev/null || printf 'OPEN\n'
}

json_append() {
    file="$1"
    object="$2"
    jq --argjson object "$object" '. + [$object]' "$file" > "$file.tmp"
    mv "$file.tmp" "$file"
}

active_paths_for_issue() {
    active_issue="$1"
    jq -c --argjson issue "$active_issue" '.[] | select(.number == $issue)' "$ACTIVE_FILE" | extract_paths
}

candidate_numbers="$(jq -r 'sort_by(.number) | .[].number' "$AUTO_FILE")"
for number in $candidate_numbers; do
    issue_json="$(jq -c --argjson number "$number" '.[] | select(.number == $number)' "$AUTO_FILE")"
    deps="$(printf '%s\n' "$issue_json" | extract_deps | sort -n | uniq)"
    unmet=""
    for dep in $deps; do
        state="$(issue_state "$dep")"
        if [ "$state" != "CLOSED" ]; then
            unmet="${unmet}${unmet:+ }${dep}"
        fi
    done
    if [ -n "$unmet" ]; then
        object="$(printf '%s\n' "$issue_json" | jq --arg unmet "$unmet" '. + {reason:"blocked_dependencies", unmet_dependencies:($unmet | split(" ") | map(tonumber))}')"
        json_append "$BLOCKED_FILE" "$object"
        continue
    fi

    candidate_paths="$(printf '%s\n' "$issue_json" | extract_paths | sort -u)"
    conflict_issue=""
    conflict_path=""
    active_numbers="$(jq -r 'sort_by(.number) | .[].number' "$ACTIVE_FILE")"
    for active in $active_numbers; do
        active_paths="$(active_paths_for_issue "$active" | sort -u)"
        for path in $candidate_paths; do
            if printf '%s\n' "$active_paths" | grep -Fx "$path" >/dev/null 2>&1; then
                conflict_issue="$active"
                conflict_path="$path"
                break 2
            fi
        done
    done

    if [ -n "$conflict_issue" ]; then
        object="$(printf '%s\n' "$issue_json" | jq --argjson conflicts_with "$conflict_issue" --arg path "$conflict_path" '. + {reason:"path_conflict", conflicts_with:$conflicts_with, path:$path}')"
        json_append "$CONFLICTS_FILE" "$object"
        continue
    fi

    object="$(printf '%s\n' "$issue_json" | jq --argjson paths "$(printf '%s\n' "$candidate_paths" | jq -R . | jq -s .)" '. + {paths:$paths}')"
    json_append "$READY_FILE" "$object"
done

jq -n \
    --slurpfile ready "$READY_FILE" \
    --slurpfile blocked "$BLOCKED_FILE" \
    --slurpfile conflicts "$CONFLICTS_FILE" \
    --slurpfile claimed "$ACTIVE_FILE" \
    --argjson batch_size "$batch_size" \
    '{
      ready: ($ready[0] | sort_by(.number)),
      blocked: ($blocked[0] | sort_by(.number)),
      claimed: ($claimed[0] | sort_by(.number)),
      conflicts: ($conflicts[0] | sort_by(.number)),
      batch: ($ready[0] | sort_by(.number) | .[:$batch_size])
    }'
