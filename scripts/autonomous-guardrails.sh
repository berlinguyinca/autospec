#!/usr/bin/env bash
# scripts/autonomous-guardrails.sh — deterministic parent guardrail helpers for
# autonomous merge safety. Issue #1543 intentionally keeps this as a small
# foundation; child issues deepen the individual mechanisms.

set -eu

SCRIPT_NAME="autonomous-guardrails"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./autospec-runtime-config.sh
. "$SCRIPT_DIR/autospec-runtime-config.sh"

usage() {
    cat <<'USAGE'
Usage:
  autonomous-guardrails.sh diff-guard --changed-files <file> [--lane implementer|verifier]
  autonomous-guardrails.sh mutation-guard --baseline <json> --current <json>
  autonomous-guardrails.sh blast-radius --changed-files <file> [--fenced-surfaces <yml>] [--json]
  autonomous-guardrails.sh separation-of-powers --lane-metadata <json> [--json]
  autonomous-guardrails.sh provenance \
      --repo OWNER/REPO --pr N --changed-files <file> --gate-evidence <json> \
      [--lane-metadata <json>] --rollback-handle <ref-or-command> --out <json>
  autonomous-guardrails.sh self-originated --pr N --repo OWNER/REPO \
      [--protected-branches csv1,csv2]

Decisions:
  diff-guard exits 1 with DECISION:block when an implementer changes test/eval-harness files.
  mutation-guard exits 1 with DECISION:block when current mutation score drops below baseline.
  blast-radius exits 1 with DECISION:quarantine when fenced/high-risk paths changed.
  separation-of-powers exits 1 when author/verifier/approver overlap or verifier prompt is not adversarial+independent.
  provenance writes autospec.autonomous.merge_provenance.v1 JSON.
  self-originated exits 1 with DECISION:block when a self-originated PR (per
  scripts/autonomous-provenance.sh resolve) targets a protected parent branch
  directly instead of the autonomous integration branch.
USAGE
}

die() { printf '%s: %s\n' "$SCRIPT_NAME" "$1" >&2; exit 2; }

require_file() {
    local path="$1" label="$2"
    [ -n "$path" ] || die "$label is required"
    [ -f "$path" ] || die "$label not found: $path"
}

read_changed_files() {
    local file="$1"
    sed '/^[[:space:]]*$/d; s#^\./##' "$file"
}

is_immutable_verifier_path() {
    case "$1" in
        tests/*|*/tests/*|test/*|*/test/*) return 0 ;;
        scripts/validate.sh|scripts/lint-issue.sh|scripts/lint-implementation.sh|scripts/verify-*|tests/verify-*|tests/fixtures/*) return 0 ;;
        skills/*/tests/*|schemas/eval-*|eval/*|evals/*|benchmarks/*) return 0 ;;
        *) return 1 ;;
    esac
}

default_fenced_surfaces_file() {
    if [ -n "${AUTOSPEC_FENCED_SURFACES_FILE:-}" ]; then
        printf '%s' "$AUTOSPEC_FENCED_SURFACES_FILE"
        return 0
    fi
    if [ -f .autospec/fenced-surfaces.yml ]; then
        printf '%s' .autospec/fenced-surfaces.yml
        return 0
    fi
    if [ -f .autospec/autospec.yml ]; then
        printf '%s' .autospec/autospec.yml
        return 0
    fi
    printf ''
}

# Legacy fallback stays in code so older repos without fenced_surfaces config
# remain fail-safe for known dangerous surfaces. New repos should configure
# fenced_surfaces in .autospec/autospec.yml or pass --fenced-surfaces.
is_high_risk_path() {
    case "$1" in
        scripts/autospec-autonomous.sh|scripts/autonomous-*.sh|scripts/autospec-autonomous-run-drain.sh) return 0 ;;
        scripts/worktree-guard.sh|scripts/claim-guard.sh|scripts/autospec-autonomy-gate.sh) return 0 ;;
        skills/autospec*/SKILL.md|skills/autospec*/codex/prompt.md|skills/autospec*/opencode/agent.md) return 0 ;;
        .github/workflows/*|install.sh|bootstrap.sh|uninstall.sh) return 0 ;;
        schemas/*|packages/*|crates/*|Cargo.toml|Cargo.lock) return 0 ;;
        trading-system/money/*|trading-system/money/**|trading-system/risk/*|trading-system/risk/**|trading-system/execution/*|trading-system/execution/**) return 0 ;;
        *migration*|*secret*|*auth*|*token*) return 0 ;;
        *) return 1 ;;
    esac
}

json_array_from_lines() {
    jq -R -s 'split("\n") | map(select(length > 0))'
}

cmd_diff_guard() {
    local changed_files="" lane="implementer"
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --lane) lane="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "diff-guard: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"
    case "$lane" in
        implementer|verifier) ;;
        *) die "diff-guard: --lane must be implementer or verifier" ;;
    esac

    local tmp
    tmp="$(mktemp "${TMPDIR:-/tmp}/autonomous-diff-guard.XXXXXX")"
    while IFS= read -r path; do
        if is_immutable_verifier_path "$path"; then
            printf '%s\n' "$path" >> "$tmp"
        fi
    done <<EOF_CHANGED
$(read_changed_files "$changed_files")
EOF_CHANGED

    if [ -s "$tmp" ] && [ "$lane" != "verifier" ]; then
        printf 'DECISION:block\n'
        printf 'REASON:immutable_verifier_modified\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        exit 1
    fi
    if [ -s "$tmp" ]; then
        printf 'DECISION:allow\n'
        printf 'REASON:verifier_lane_bypass\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        return 0
    fi
    rm -f "$tmp"
    printf 'DECISION:allow\n'
}

cmd_mutation_guard() {
    local baseline="" current=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --baseline) baseline="${2:-}"; shift 2 ;;
            --current) current="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "mutation-guard: unknown option: $1" ;;
        esac
    done
    require_file "$baseline" "--baseline"
    require_file "$current" "--current"

    local baseline_score current_score mutants
    baseline_score="$(jq -r 'if has("score") then .score else ((.killed // 0) * 100 / (.total // 1) | floor) end' "$baseline")"
    current_score="$(jq -r 'if has("score") then .score else ((.killed // 0) * 100 / (.total // 1) | floor) end' "$current")"

    case "$baseline_score:$current_score" in
        *null*|*:*null*|*[^0-9.:]*) die "mutation-guard: scores must be numeric or derivable from killed/total" ;;
    esac

    if awk "BEGIN { exit !($current_score < $baseline_score) }"; then
        printf 'DECISION:block\n'
        printf 'REASON:mutation_score_regression\n'
        printf 'BASELINE:%s\n' "$baseline_score"
        printf 'CURRENT:%s\n' "$current_score"
        mutants="$(jq -r '
            (.surviving_mutants // .survivors // .mutants // [])[]
            | select((.status // "survived") != "killed")
            | "MUTANT:" + ((.id // .name // "unknown")|tostring)
              + ":" + ((.file // .path // "unknown")|tostring)
              + ":" + ((.line // 0)|tostring)
              + ":" + ((.description // .mutator // .operator // "surviving mutant")|tostring)
        ' "$current")"
        if [ -n "$mutants" ]; then
            printf '%s\n' "$mutants"
        else
            printf 'MUTANT:unknown:unknown:0:mutation score regressed without survivor details\n'
        fi
        exit 1
    fi

    printf 'DECISION:allow\n'
    printf 'BASELINE:%s\n' "$baseline_score"
    printf 'CURRENT:%s\n' "$current_score"
}

cmd_blast_radius() {
    local changed_files="" fenced_surfaces="" json=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --fenced-surfaces) fenced_surfaces="${2:-}"; shift 2 ;;
            --json) json=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *) die "blast-radius: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"
    if [ -z "$fenced_surfaces" ]; then
        fenced_surfaces="$(default_fenced_surfaces_file)"
    fi

    local result status dir
    dir="$(cd "$(dirname "$0")" && pwd)"
    result="$(python3 "$dir/blast-radius-classifier.py" --changed-files "$changed_files" --fenced-surfaces "$fenced_surfaces")"
    status="$(printf '%s' "$result" | jq -r '.exit_status')"
    if [ "$json" -eq 1 ]; then
        printf '%s\n' "$result" | jq 'del(.exit_status)'
        exit "$status"
    fi
    if [ "$status" -ne 0 ]; then
        printf 'DECISION:quarantine\n'
        printf 'REASON:%s\n' "$(printf '%s' "$result" | jq -r '.reason')"
        printf '%s\n' "$result" | jq -r '.fenced_matches[]? | "SURFACE:" + .surface + ":" + .path + ":" + .reason'
        printf '%s\n' "$result" | jq -r '.paths[]? | "PATH:" + .'
        exit 1
    fi
    printf 'DECISION:allow\n'
    printf 'LABEL:%s\n' "$(printf '%s' "$result" | jq -r '.label')"
}

blast_radius_json() {
    local changed_files="$1" fenced_surfaces="${2:-}"
    if [ -z "$fenced_surfaces" ]; then
        fenced_surfaces="$(default_fenced_surfaces_file)"
    fi
    local dir
    dir="$(cd "$(dirname "$0")" && pwd)"
    python3 "$dir/blast-radius-classifier.py" --changed-files "$changed_files" --fenced-surfaces "$fenced_surfaces" | jq 'del(.exit_status)'
}

separation_of_powers_json() {
    local lane_metadata="$1"
    jq -c '
        def actor($k):
          (.[$k] // {}) as $v
          | if ($v|type) == "object" then {
              lane: (($v.lane // $v.name // $v.id // "")|tostring),
              agent_id: (($v.agent_id // $v.agent // $v.context_id // $v.id // $v.lane // "")|tostring)
            }
            else {lane: ($v|tostring), agent_id: ($v|tostring)} end;
        def norm: ascii_downcase | gsub("[^a-z0-9]+"; "-") | gsub("(^-)|(-$)"; "");
        (actor("author")) as $author
        | (actor("verifier")) as $verifier
        | (actor("approver")) as $approver
        | (.verifier_prompt // {}) as $prompt
        | (($prompt.mode // "")|tostring|ascii_downcase) as $mode
        | (($prompt.context // "")|tostring|ascii_downcase) as $context
        | (($prompt.text // $prompt.body // "")|tostring|ascii_downcase) as $text
        | [
            (if (($author.agent_id|norm) == "" or ($verifier.agent_id|norm) == "" or ($approver.agent_id|norm) == "") then "missing_lane_identity" else empty end),
            (if (($author.agent_id|norm) != "" and ($author.agent_id|norm) == ($verifier.agent_id|norm)) or (($author.lane|norm) != "" and ($author.lane|norm) == ($verifier.lane|norm)) then "author_verifier_overlap" else empty end),
            (if (($author.agent_id|norm) != "" and ($author.agent_id|norm) == ($approver.agent_id|norm)) or (($author.lane|norm) != "" and ($author.lane|norm) == ($approver.lane|norm)) then "author_approver_overlap" else empty end),
            (if (($verifier.agent_id|norm) != "" and ($verifier.agent_id|norm) == ($approver.agent_id|norm)) or (($verifier.lane|norm) != "" and ($verifier.lane|norm) == ($approver.lane|norm)) then "verifier_approver_overlap" else empty end),
            (if (($mode|test("adversarial|refute")) or ($text|test("adversarial|refute"))) then empty else "verifier_prompt_not_adversarial" end),
            (if (($context|test("independent")) or ($text|test("independent"))) then empty else "verifier_prompt_not_independent" end)
          ] as $violations
        | {
            schema: "autospec.autonomous.separation_of_powers.v1",
            decision: (if ($violations|length) == 0 then "allow" else "block" end),
            reason: (if ($violations|length) == 0 then "distinct_adversarial_lanes" else "separation_of_powers_violation" end),
            violations: $violations,
            lane_metadata: {author:$author, verifier:$verifier, approver:$approver},
            verifier_prompt: {mode:$mode, context:$context}
          }
    ' "$lane_metadata"
}

emit_separation_of_powers() {
    local result="$1"
    if [ "$(printf '%s' "$result" | jq -r '.decision')" = "allow" ]; then
        printf 'DECISION:allow\n'
        printf 'REASON:distinct_adversarial_lanes\n'
        return 0
    fi
    printf 'DECISION:block\n'
    printf 'REASON:separation_of_powers_violation\n'
    printf '%s\n' "$result" | jq -r '.violations[] | "VIOLATION:" + .'
}

cmd_separation_of_powers() {
    local lane_metadata="" json=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --lane-metadata) lane_metadata="${2:-}"; shift 2 ;;
            --json) json=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *) die "separation-of-powers: unknown option: $1" ;;
        esac
    done
    require_file "$lane_metadata" "--lane-metadata"

    local result
    if ! result="$(separation_of_powers_json "$lane_metadata")"; then
        die "separation-of-powers: invalid JSON: $lane_metadata"
    fi

    if [ "$json" -eq 1 ]; then
        printf '%s\n' "$result"
    else
        emit_separation_of_powers "$result"
    fi

    [ "$(printf '%s' "$result" | jq -r '.decision')" = "allow" ]
}

cmd_provenance() {
    local repo="" pr="" changed_files="" gate_evidence="" rollback_handle="" out="" lane_metadata=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            --pr) pr="${2:-}"; shift 2 ;;
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --gate-evidence) gate_evidence="${2:-}"; shift 2 ;;
            --rollback-handle) rollback_handle="${2:-}"; shift 2 ;;
            --lane-metadata) lane_metadata="${2:-}"; shift 2 ;;
            --out) out="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "provenance: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "provenance: --repo is required"
    [ -n "$pr" ] || die "provenance: --pr is required"
    [ -n "$rollback_handle" ] || die "provenance: --rollback-handle is required"
    [ -n "$out" ] || die "provenance: --out is required"
    require_file "$changed_files" "--changed-files"
    require_file "$gate_evidence" "--gate-evidence"
    local changed_json evidence_json blast_json generated_at separation_json lane_metadata_json
    changed_json="$(read_changed_files "$changed_files" | json_array_from_lines)"
    evidence_json="$(cat "$gate_evidence")"
    blast_json="$(blast_radius_json "$changed_files")"
    if [ -n "$lane_metadata" ]; then
        separation_json="$(cmd_separation_of_powers --lane-metadata "$lane_metadata" --json)"
        lane_metadata_json="$(printf '%s' "$separation_json" | jq -c '.lane_metadata')"
    else
        separation_json='{"schema":"autospec.autonomous.separation_of_powers.v1","decision":"not_recorded","reason":"lane_metadata_not_provided","violations":[]}'
        lane_metadata_json='null'
    fi
    generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    mkdir -p "$(dirname "$out")"
    jq -n \
        --arg schema 'autospec.autonomous.merge_provenance.v1' \
        --arg repo "$repo" \
        --argjson pr "$pr" \
        --arg generated_at "$generated_at" \
        --arg rollback_handle "$rollback_handle" \
        --argjson changed_files "$changed_json" \
        --argjson gate_evidence "$evidence_json" \
        --argjson blast_radius "$blast_json" \
        --argjson separation_of_powers "$separation_json" \
        --argjson lane_metadata "$lane_metadata_json" \
        '{schema:$schema, repo:$repo, pr:$pr, generated_at:$generated_at, rollback_handle:$rollback_handle, changed_files:$changed_files, gate_evidence:$gate_evidence, blast_radius:$blast_radius, separation_of_powers:$separation_of_powers, lane_metadata:$lane_metadata}' \
        > "$out"
    printf 'DECISION:provenance-written\n'
    printf 'PROVENANCE:%s\n' "$out"
}

# cmd_self_originated — defense-in-depth premerge check (issue #1742,
# docs/specs/2026-07-10-autonomous-integration-branch-design.md §Architecture
# item 6). Blocks a self-originated PR (per autonomous-provenance.sh resolve)
# that targets a protected parent branch directly, instead of the autonomous
# integration branch. Fails CLOSED: any lookup ambiguity (unresolved base ref,
# no linked issue) is treated as a block on a protected base, mirroring
# autonomous-provenance.sh's own fail-closed-to-self posture.
cmd_self_originated() {
    local pr="" repo="" protected_branches=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --pr) pr="${2:-}"; shift 2 ;;
            --repo) repo="${2:-}"; shift 2 ;;
            --protected-branches) protected_branches="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "self-originated: unknown option: $1" ;;
        esac
    done
    [ -n "$pr" ] || die "self-originated: --pr is required"
    [ -n "$repo" ] || die "self-originated: --repo is required"

    local provenance_sh
    provenance_sh="$SCRIPT_DIR/autonomous-provenance.sh"

    local base
    base="$(gh pr view "$pr" --repo "$repo" --json baseRefName --jq '.baseRefName' 2>/dev/null || true)"
    if [ -z "$base" ]; then
        # Base ref could not be resolved (gh/API unavailable) — fail CLOSED,
        # like every enforcement layer on this surface: an API hiccup must
        # not let a self-originated PR slip onto a protected parent. The
        # check is opt-in (--check-self-originated in the premerge gate), so
        # callers that never asked for it are unaffected.
        printf 'DECISION:block\n'
        printf 'REASON:self_originated_direct_merge\n'
        printf 'DETAIL:base ref could not be resolved for PR %s\n' "$pr"
        exit 1
    fi

    # Integration branches (prefix match) are never protected, regardless of
    # provenance — that is exactly where self-originated work is meant to land.
    local integration_prefix
    integration_prefix="$(autospec_runtime_config_get "autonomous.self_originated.integration_branch_prefix" "autospec/autonomous-")"
    case "$base" in
        "$integration_prefix"*)
            printf 'DECISION:allow\n'
            printf 'REASON:integration_branch\n'
            return 0
            ;;
    esac

    local default_branch configured_protected combined_csv
    default_branch="$(gh repo view "$repo" --json defaultBranchRef --jq '.defaultBranchRef.name' 2>/dev/null || true)"
    if [ -z "$default_branch" ]; then
        # Default branch could not be resolved — fail CLOSED: without it the
        # protected set is unknowable, so treat the base as protected and
        # let provenance (itself fail-closed to `self`) decide below.
        default_branch="$base"
    fi
    configured_protected="$(autospec_runtime_config_get "autonomous.self_originated.protected_branches" "")"
    combined_csv="$default_branch"
    if [ -n "$configured_protected" ]; then
        combined_csv="${combined_csv:+$combined_csv,}$configured_protected"
    fi
    if [ -n "$protected_branches" ]; then
        combined_csv="${combined_csv:+$combined_csv,}$protected_branches"
    fi

    local is_protected=1 candidate old_ifs
    old_ifs="$IFS"
    IFS=','
    for candidate in $combined_csv; do
        IFS="$old_ifs"
        [ -n "$candidate" ] || continue
        if [ "$candidate" = "$base" ]; then
            is_protected=0
            break
        fi
        IFS=','
    done
    IFS="$old_ifs"

    if [ "$is_protected" -ne 0 ]; then
        printf 'DECISION:allow\n'
        printf 'REASON:base_not_protected\n'
        return 0
    fi

    # Base is a protected parent branch — provenance decides the rest.
    local issue
    issue="$(gh pr view "$pr" --repo "$repo" --json closingIssuesReferences --jq '.closingIssuesReferences[0].number // empty' 2>/dev/null || true)"
    if [ -z "$issue" ]; then
        # No linked issue to resolve provenance from — fail closed to block
        # on a protected base rather than guess.
        printf 'DECISION:block\n'
        printf 'REASON:self_originated_direct_merge\n'
        printf 'BASE:%s\n' "$base"
        printf 'DETAIL:no linked issue found to resolve provenance\n'
        exit 1
    fi

    local provenance
    provenance="$(bash "$provenance_sh" resolve --issue "$issue" --repo "$repo" 2>/dev/null || printf 'self')"
    if [ "$provenance" != "self" ]; then
        printf 'DECISION:allow\n'
        printf 'REASON:operator_originated\n'
        printf 'ISSUE:%s\n' "$issue"
        return 0
    fi

    local allow_direct
    allow_direct="$(autospec_runtime_config_get "autonomous.self_originated.allow_direct_merge" "false")"
    case "$allow_direct" in
        true|True|TRUE|1)
            printf 'DECISION:allow\n'
            printf 'REASON:allow_direct_merge_override\n'
            printf 'BASE:%s\n' "$base"
            printf 'ISSUE:%s\n' "$issue"
            return 0
            ;;
    esac

    printf 'DECISION:block\n'
    printf 'REASON:self_originated_direct_merge\n'
    printf 'BASE:%s\n' "$base"
    printf 'ISSUE:%s\n' "$issue"
    exit 1
}

main() {
    [ "$#" -gt 0 ] || { usage >&2; exit 2; }
    local sub="$1"; shift
    case "$sub" in
        diff-guard) cmd_diff_guard "$@" ;;
        mutation-guard) cmd_mutation_guard "$@" ;;
        blast-radius) cmd_blast_radius "$@" ;;
        separation-of-powers) cmd_separation_of_powers "$@" ;;
        provenance) cmd_provenance "$@" ;;
        self-originated) cmd_self_originated "$@" ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown subcommand: $sub" ;;
    esac
}

main "$@"
