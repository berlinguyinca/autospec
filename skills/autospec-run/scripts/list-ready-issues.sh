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
batch_size=1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAFETY_GATE="$SCRIPT_DIR/issue-safety-gate.sh"
RUN_STATE="$SCRIPT_DIR/run-state.sh"
HEARTBEAT_READ="$SCRIPT_DIR/heartbeat-read.sh"
[ -f "$SAFETY_GATE" ] || die "missing issue safety gate helper: $SAFETY_GATE"
# shellcheck source=/dev/null
. "$SAFETY_GATE"
if [ -f "$SCRIPT_DIR/../../../scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/../../../scripts/autospec-runtime-config.sh"
elif [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

if command -v autospec_runtime_repo_workers >/dev/null 2>&1; then
    max_repo_workers="$(autospec_runtime_repo_workers)"
else
    max_repo_workers="${AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS:-0}"
fi

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --batch-size) batch_size="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$batch_size" in *[!0-9]*|'') die "--batch-size must be an integer" ;; esac
[ "$batch_size" -gt 0 ] || batch_size=1
case "$max_repo_workers" in *[!0-9]*|'') max_repo_workers=0 ;; esac

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
        --json number,title,body,labels,author
}

AUTO_FILE="$(mktemp -t autospec-auto-issues.XXXXXX)"
ACTIVE_FILE="$(mktemp -t autospec-active-issues.XXXXXX)"
READY_FILE="$(mktemp -t autospec-ready.XXXXXX)"
BLOCKED_FILE="$(mktemp -t autospec-blocked.XXXXXX)"
CONFLICTS_FILE="$(mktemp -t autospec-conflicts.XXXXXX)"
trap 'rm -f "$AUTO_FILE" "$ACTIVE_FILE" "$READY_FILE" "$BLOCKED_FILE" "$CONFLICTS_FILE"' EXIT

issue_list auto-implement > "$AUTO_FILE"

# AUTOSPEC_RUN_ONLY_ISSUES scopes the ready/batch queue to a space-separated
# issue-number list (set by the conductor's dispatch-time provenance split —
# scripts/lib/autospec-loop.sh — so the operator and self-originated batches
# each drain only their own subset instead of the full ready queue). Unset or
# empty keeps today's full-queue behavior (fail-open to unconstrained).
if [ -n "${AUTOSPEC_RUN_ONLY_ISSUES:-}" ]; then
    only_json="$(printf '%s\n' "$AUTOSPEC_RUN_ONLY_ISSUES" | tr ' ' '\n' | awk 'NF' | jq -R 'tonumber' | jq -s .)"
    jq --argjson only "$only_json" '[.[] | select(([.number] - $only | length) == 0)]' \
        "$AUTO_FILE" > "$AUTO_FILE.tmp"
    mv "$AUTO_FILE.tmp" "$AUTO_FILE"
fi

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

# Emit only the body lines inside the "## Dependencies" section (heading
# exclusive), stopping at the next "## " heading or end of body. Mirrors
# scripts/lint-issue.sh extract_section so dep parsing matches the authoritative
# section convention. Scoping is required so prose "#B depends on #A" outside the
# section (e.g. a "## Shared contracts" sequencing note) is never read as a real
# dependency edge (issue #1663).
dependencies_section() {
    awk '
      $0 == "## Dependencies" { in_section=1; next }
      in_section && /^## / { exit }
      in_section { print }
    '
}

extract_deps() {
    jq -r '.body // ""' | dependencies_section | grep -Eoi 'Depends on[[:space:]]+(issue[[:space:]]+)?#?[0-9]+' | grep -Eo '[0-9]+' || true
}

issue_view_json() {
    gh issue view "$1" --repo "$repo" --json state,body,labels 2>/dev/null \
        || printf '{"state":"OPEN","body":"","labels":[]}\n'
}

dep_label_is_non_blocking() {
    target_json="$1"
    labels="${AUTOSPEC_NON_BLOCKING_DEP_LABELS:-epic umbrella}"
    printf '%s\n' "$target_json" \
        | jq -e --arg labels "$labels" '
            ($labels | gsub(","; " ") | split(" ") | map(select(length > 0) | ascii_downcase)) as $allowed
            | any((.labels // [])[]?.name; (. | ascii_downcase) as $name | $allowed | index($name))
          ' >/dev/null 2>&1
}

target_tracks_dependent() {
    target_json="$1"
    dependent="$2"
    printf '%s\n' "$target_json" | jq -r '.body // ""' | awk -v issue="$dependent" '
      function header_name(line) {
        sub(/^##[[:space:]]*/, "", line)
        sub(/[[:space:]]*#*[[:space:]]*$/, "", line)
        return tolower(line)
      }
      /^##[[:space:]]+/ {
        h=header_name($0)
        in_tracker=(h ~ /^(children|child issues|tasks|task list|subtasks)$/)
        next
      }
      in_tracker && $0 ~ ("#" issue "([^0-9]|$)") && $0 ~ /^[[:space:]]*[-*][[:space:]]*(\[[ xX]\][[:space:]]*)?/ {
        found=1
      }
      END { exit(found ? 0 : 1) }
    '
}

dependency_path_reaches() {
    start="$1"
    target="$2"
    target_json="$3"
    START_ISSUE="$start" TARGET_ISSUE="$target" TARGET_JSON="$target_json" AUTO_FILE="$AUTO_FILE" python3 <<'PY'
import json
import os
import re
import sys

dep_re = re.compile(r"Depends on\s+(?:issue\s+)?#?(\d+)", re.IGNORECASE)
start = int(os.environ["START_ISSUE"])
target = int(os.environ["TARGET_ISSUE"])


def dependencies_section(body):
    """Return only the lines inside the "## Dependencies" section (heading
    exclusive), stopping at the next "## " heading. Mirrors the shell
    dependencies_section() so prose "depends on #N" outside the section never
    forms a phantom edge in cycle detection (issue #1663)."""
    lines = []
    in_section = False
    for line in body.splitlines():
        if line == "## Dependencies":
            in_section = True
            continue
        if in_section and line.startswith("## "):
            break
        if in_section:
            lines.append(line)
    return "\n".join(lines)


with open(os.environ["AUTO_FILE"], encoding="utf-8") as handle:
    issues = {int(item["number"]): item.get("body") or "" for item in json.load(handle)}

try:
    target_json = json.loads(os.environ.get("TARGET_JSON") or "{}")
except json.JSONDecodeError:
    target_json = {}
issues.setdefault(start, target_json.get("body") or "")

seen = set()

def deps_for(issue):
    return [
        int(match)
        for match in dep_re.findall(dependencies_section(issues.get(issue, "")))
    ]

def reaches(issue):
    if issue in seen:
        return False
    seen.add(issue)
    for dep in deps_for(issue):
        if dep == target or reaches(dep):
            return True
    return False

sys.exit(0 if reaches(start) else 1)
PY
}

json_array_from_words() {
    words="$1"
    if [ -z "$words" ]; then
        printf '[]'
    else
        printf '%s\n' "$words" | tr ' ' '\n' | awk 'NF' | jq -R 'tonumber' | jq -s .
    fi
}

json_append() {
    file="$1"
    object="$2"
    jq --argjson object "$object" '. + [$object]' "$file" > "$file.tmp"
    mv "$file.tmp" "$file"
}

iso_to_epoch() {
    [ -n "$1" ] || return 0
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s 2>/dev/null \
        || date -u -d "$1" +%s 2>/dev/null \
        || true
}

issue_heartbeat_exists() {
    issue_number="$1"
    [ -x "$HEARTBEAT_READ" ] || return 1
    heartbeat_json="$("$HEARTBEAT_READ" --issue "$issue_number" --repo "$repo" 2>/dev/null || true)"
    [ -n "$heartbeat_json" ]
}

issue_run_state() {
    issue_number="$1"
    [ -x "$RUN_STATE" ] || return 0
    "$RUN_STATE" read --issue "$issue_number" --repo "$repo" 2>/dev/null
}

branch_ref_exists() {
    branch_name="$1"
    [ -n "$branch_name" ] || return 1
    git show-ref --verify --quiet "refs/heads/$branch_name" 2>/dev/null && return 0
    git show-ref --verify --quiet "refs/remotes/origin/$branch_name" 2>/dev/null && return 0
    remote_ref="$(git ls-remote --heads origin "$branch_name" 2>/dev/null)" || return 2
    [ -n "$remote_ref" ]
}

startup_claim_has_evidence() {
    issue_number="$1"
    state_json="$2"
    issue_heartbeat_exists "$issue_number" && return 0
    branch_name="$(printf '%s\n' "$state_json" | jq -r '.branch // empty' 2>/dev/null || true)"
    if [ -n "$branch_name" ]; then
        branch_status=0
        branch_ref_exists "$branch_name" || branch_status="$?"
        [ "$branch_status" -eq 0 ] && return 0
        [ "$branch_status" -eq 2 ] && return 0
    fi
    pr_number="$(printf '%s\n' "$state_json" | jq -r '.pr // empty' 2>/dev/null || true)"
    [ -n "$pr_number" ] && return 0
    return 1
}

release_startup_claim_if_stale() {
    issue_number="$1"
    state_json="$2"
    release_startup_claim_result="preserve"
    timeout=300

    updated_at="$(printf '%s\n' "$state_json" | jq -r '.updated_at // .claimed_at // empty' 2>/dev/null || true)"
    updated_epoch="$(iso_to_epoch "$updated_at")"
    if [ -n "$updated_epoch" ]; then
        now_epoch="$(date -u +%s)"
        age=$((now_epoch - updated_epoch))
    else
        age="$timeout"
    fi
    if [ "$age" -lt "$timeout" ]; then
        release_startup_claim_result="ignore"
        return 0
    fi

    if ! gh issue edit "$issue_number" --repo "$repo" \
        --remove-label in-progress-by-bot \
        --add-label auto-implement >/dev/null 2>&1; then
        return 1
    fi
    if [ -x "$RUN_STATE" ]; then
        if ! "$RUN_STATE" clear --issue "$issue_number" --repo "$repo" >/dev/null 2>&1; then
            gh issue edit "$issue_number" --repo "$repo" \
                --remove-label auto-implement \
                --add-label in-progress-by-bot >/dev/null 2>&1 || true
            return 1
        fi
    fi
    release_startup_claim_result="released"
    return 0
}

filter_active_startup_claims() {
    filtered_file="$(mktemp -t autospec-active-filtered.XXXXXX)"
    printf '[]\n' > "$filtered_file"
    active_numbers="$(jq -r 'sort_by(.number) | .[].number' "$ACTIVE_FILE")"
    for active_number in $active_numbers; do
        active_object="$(jq -c --argjson number "$active_number" '.[] | select(.number == $number)' "$ACTIVE_FILE")"
        if ! state_json="$(issue_run_state "$active_number")"; then
            json_append "$filtered_file" "$active_object"
            continue
        fi
        if startup_claim_has_evidence "$active_number" "$state_json"; then
            json_append "$filtered_file" "$active_object"
            continue
        fi
        release_startup_claim_result="preserve"
        if ! release_startup_claim_if_stale "$active_number" "$state_json"; then
            json_append "$filtered_file" "$active_object"
        elif [ "$release_startup_claim_result" = "preserve" ]; then
            json_append "$filtered_file" "$active_object"
        fi
    done
    mv "$filtered_file" "$ACTIVE_FILE"
}

serialization_reasons_for_issue() {
    jq -r '
      [ (.labels // [])[]?.name
        | select(. == "reasoning:deep" or . == "priority:high" or . == "regression" or . == "audit" or . == "release") ]
      | unique
      | .[]
    '
}

active_paths_for_issue() {
    active_issue="$1"
    jq -c --argjson issue "$active_issue" '.[] | select(.number == $issue)' "$ACTIVE_FILE" | extract_paths
}

ready_paths_for_issue() {
    ready_issue="$1"
    jq -r --argjson issue "$ready_issue" '.[] | select(.number == $issue) | (.paths // [])[]' "$READY_FILE"
}

issue_list in-progress-by-bot > "$ACTIVE_FILE"
filter_active_startup_claims
active_count="$(jq 'length' "$ACTIVE_FILE")"
if [ "$max_repo_workers" -gt 0 ]; then
    remaining_workers=$((max_repo_workers - active_count))
    [ "$remaining_workers" -gt 0 ] || remaining_workers=0
else
    remaining_workers="$batch_size"
fi
if [ "$max_repo_workers" -gt 0 ] && [ "$active_count" -ge "$max_repo_workers" ]; then
    effective_batch_size=0
elif [ "$max_repo_workers" -gt 0 ] && [ "$batch_size" -gt "$remaining_workers" ]; then
    effective_batch_size="$remaining_workers"
else
    effective_batch_size="$batch_size"
fi

candidate_numbers="$(jq -r 'sort_by(.number) | .[].number' "$AUTO_FILE")"
for number in $candidate_numbers; do
    issue_json="$(jq -c --argjson number "$number" '.[] | select(.number == $number)' "$AUTO_FILE")"
    if printf '%s\n' "$issue_json" | jq -e 'any((.labels // [])[]?.name; . == "autospec:needs-human")' >/dev/null 2>&1; then
        object="$(printf '%s\n' "$issue_json" | jq '. + {reason:"autospec_needs_human", blocked_label:"autospec:needs-human"}')"
        json_append "$BLOCKED_FILE" "$object"
        continue
    fi

    safety_gate_result="$(printf '%s\n' "$issue_json" | autospec_issue_safety_gate_result)"
    if ! printf '%s\n' "$safety_gate_result" | jq -e '.ok == true' >/dev/null 2>&1; then
        object="$(printf '%s\n' "$issue_json" | jq --argjson safety_gate "$safety_gate_result" '. + {reason:"safety_gate_failed", safety_gate:$safety_gate}')"
        json_append "$BLOCKED_FILE" "$object"
        continue
    fi

    deps="$(printf '%s\n' "$issue_json" | extract_deps | sort -n | uniq)"
    unmet=""
    cycle_deps=""
    non_blocking_refs="[]"
    for dep in $deps; do
        target_json="$(issue_view_json "$dep")"
        state="$(printf '%s\n' "$target_json" | jq -r '.state // "OPEN"' 2>/dev/null || printf 'OPEN')"
        if [ "$state" != "CLOSED" ]; then
            non_blocking_reason=""
            non_blocking_cycle="false"
            if dep_label_is_non_blocking "$target_json"; then
                non_blocking_reason="epic_label"
            elif target_tracks_dependent "$target_json" "$number"; then
                non_blocking_reason="children_back_edge"
                non_blocking_cycle="true"
            fi
            if [ -n "$non_blocking_reason" ]; then
                ref="$(jq -n \
                    --argjson issue "$dep" \
                    --arg reason "$non_blocking_reason" \
                    --argjson cycle "$non_blocking_cycle" \
                    '{issue:$issue, reason:$reason, cycle:$cycle}')"
                non_blocking_refs="$(printf '%s\n' "$non_blocking_refs" | jq --argjson ref "$ref" '. + [$ref]')"
                continue
            fi
            if dependency_path_reaches "$dep" "$number" "$target_json"; then
                cycle_deps="${cycle_deps}${cycle_deps:+ }${dep}"
            fi
            unmet="${unmet}${unmet:+ }${dep}"
        fi
    done
    if [ -n "$unmet" ]; then
        unmet_json="$(json_array_from_words "$unmet")"
        cycle_json="$(json_array_from_words "$cycle_deps")"
        reason="blocked_dependencies"
        if [ -n "$cycle_deps" ]; then
            reason="blocked_cycle"
        fi
        object="$(printf '%s\n' "$issue_json" | jq \
            --arg reason "$reason" \
            --argjson unmet "$unmet_json" \
            --argjson cycles "$cycle_json" \
            --argjson non_blocking_refs "$non_blocking_refs" \
            '. + {
              reason:$reason,
              unmet_dependencies:$unmet,
              non_blocking_refs:$non_blocking_refs
            } + (if ($cycles | length) > 0 then {cycle_dependencies:$cycles} else {} end)')"
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

    ready_numbers="$(jq -r 'sort_by(.number) | .[].number' "$READY_FILE")"
    for ready in $ready_numbers; do
        ready_paths="$(ready_paths_for_issue "$ready" | sort -u)"
        for path in $candidate_paths; do
            if printf '%s\n' "$ready_paths" | grep -Fx "$path" >/dev/null 2>&1; then
                conflict_issue="$ready"
                conflict_path="$path"
                break 2
            fi
        done
    done

    if [ -n "$conflict_issue" ]; then
        object="$(printf '%s\n' "$issue_json" | jq --argjson conflicts_with "$conflict_issue" --arg path "$conflict_path" '. + {reason:"batch_path_conflict", conflicts_with:$conflicts_with, path:$path}')"
        json_append "$CONFLICTS_FILE" "$object"
        continue
    fi

    serialization_reasons="$(printf '%s\n' "$issue_json" | serialization_reasons_for_issue)"
    serialization_reasons_json="$(printf '%s\n' "$serialization_reasons" | awk 'NF' | jq -R . | jq -s .)"
    object="$(printf '%s\n' "$issue_json" | jq \
        --argjson paths "$(printf '%s\n' "$candidate_paths" | jq -R . | jq -s .)" \
        --argjson non_blocking_refs "$non_blocking_refs" \
        --argjson serialization_reasons "$serialization_reasons_json" \
        '. + {
          paths:$paths,
          non_blocking_refs:$non_blocking_refs,
          serialization_reasons:$serialization_reasons,
          parallel_safe:(($serialization_reasons | length) == 0)
        }')"
    json_append "$READY_FILE" "$object"
done

jq -n \
    --slurpfile ready "$READY_FILE" \
    --slurpfile blocked "$BLOCKED_FILE" \
    --slurpfile conflicts "$CONFLICTS_FILE" \
    --slurpfile claimed "$ACTIVE_FILE" \
    --argjson batch_size "$batch_size" \
    --argjson effective_batch_size "$effective_batch_size" \
    --argjson max_repo_workers "$max_repo_workers" \
    --argjson active_count "$active_count" \
    --argjson remaining_workers "$remaining_workers" \
    '{
      ready: ($ready[0] | sort_by(.number)),
      blocked: ($blocked[0] | sort_by(.number)),
      claimed: ($claimed[0] | sort_by(.number)),
      conflicts: ($conflicts[0] | sort_by(.number)),
      worker_cap: {
        max_repo_workers: $max_repo_workers,
        active_count: $active_count,
        remaining: $remaining_workers,
        reached: (($max_repo_workers > 0) and ($active_count >= $max_repo_workers))
      },
      batch: (
        ($ready[0] | sort_by(.number)) as $sorted
        | if $effective_batch_size == 0 then []
          elif ($sorted[0].parallel_safe == false) then [$sorted[0]]
          else [$sorted[] | select(.parallel_safe != false)][: $effective_batch_size]
          end
      )
    }'
