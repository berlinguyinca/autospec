#!/usr/bin/env bash
# scripts/autonomous-guardrails.sh — deterministic parent guardrail helpers for
# autonomous merge safety. Issue #1543 intentionally keeps this as a small
# foundation; child issues deepen the individual mechanisms.

set -eu

SCRIPT_NAME="autonomous-guardrails"

usage() {
    cat <<'USAGE'
Usage:
  autonomous-guardrails.sh diff-guard --changed-files <file>
  autonomous-guardrails.sh blast-radius --changed-files <file>
  autonomous-guardrails.sh provenance \
      --repo OWNER/REPO --pr N --changed-files <file> --gate-evidence <json> \
      --rollback-handle <ref-or-command> --out <json>

Decisions:
  diff-guard exits 1 with DECISION:block when test/eval-harness files changed.
  blast-radius exits 1 with DECISION:block when fenced/high-risk paths changed.
  provenance writes autospec.autonomous.merge_provenance.v1 JSON.
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

is_high_risk_path() {
    case "$1" in
        scripts/autospec-autonomous.sh|scripts/autonomous-*.sh|scripts/autospec-autonomous-run-drain.sh) return 0 ;;
        scripts/worktree-guard.sh|scripts/claim-guard.sh|scripts/autospec-autonomy-gate.sh) return 0 ;;
        skills/autospec*/SKILL.md|skills/autospec*/codex/prompt.md|skills/autospec*/opencode/agent.md) return 0 ;;
        .github/workflows/*|install.sh|bootstrap.sh|uninstall.sh) return 0 ;;
        schemas/*|packages/*|crates/*|Cargo.toml|Cargo.lock) return 0 ;;
        *migration*|*secret*|*auth*|*token*) return 0 ;;
        *) return 1 ;;
    esac
}

json_array_from_lines() {
    jq -R -s 'split("\n") | map(select(length > 0))'
}

cmd_diff_guard() {
    local changed_files=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "diff-guard: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"

    local tmp
    tmp="$(mktemp "${TMPDIR:-/tmp}/autonomous-diff-guard.XXXXXX")"
    while IFS= read -r path; do
        if is_immutable_verifier_path "$path"; then
            printf '%s\n' "$path" >> "$tmp"
        fi
    done <<EOF_CHANGED
$(read_changed_files "$changed_files")
EOF_CHANGED

    if [ -s "$tmp" ]; then
        printf 'DECISION:block\n'
        printf 'REASON:immutable_verifier_modified\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        exit 1
    fi
    rm -f "$tmp"
    printf 'DECISION:allow\n'
}

cmd_blast_radius() {
    local changed_files=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "blast-radius: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"

    local tmp
    tmp="$(mktemp "${TMPDIR:-/tmp}/autonomous-blast-radius.XXXXXX")"
    while IFS= read -r path; do
        if is_high_risk_path "$path"; then
            printf '%s\n' "$path" >> "$tmp"
        fi
    done <<EOF_CHANGED
$(read_changed_files "$changed_files")
EOF_CHANGED

    if [ -s "$tmp" ]; then
        printf 'DECISION:block\n'
        printf 'REASON:high_risk_blast_radius\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        exit 1
    fi
    rm -f "$tmp"
    printf 'DECISION:allow\n'
}

blast_radius_json() {
    local changed_files="$1"
    local tmp
    tmp="$(mktemp "${TMPDIR:-/tmp}/autonomous-blast-radius-json.XXXXXX")"
    while IFS= read -r path; do
        if is_high_risk_path "$path"; then
            printf '%s\n' "$path" >> "$tmp"
        fi
    done <<EOF_CHANGED
$(read_changed_files "$changed_files")
EOF_CHANGED
    if [ -s "$tmp" ]; then
        jq -n --argjson paths "$(json_array_from_lines < "$tmp")" '{decision:"block", reason:"high_risk_blast_radius", paths:$paths}'
    else
        jq -n '{decision:"allow", reason:null, paths:[]}'
    fi
    rm -f "$tmp"
}

cmd_provenance() {
    local repo="" pr="" changed_files="" gate_evidence="" rollback_handle="" out=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            --pr) pr="${2:-}"; shift 2 ;;
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --gate-evidence) gate_evidence="${2:-}"; shift 2 ;;
            --rollback-handle) rollback_handle="${2:-}"; shift 2 ;;
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

    local changed_json evidence_json blast_json generated_at
    changed_json="$(read_changed_files "$changed_files" | json_array_from_lines)"
    evidence_json="$(cat "$gate_evidence")"
    blast_json="$(blast_radius_json "$changed_files")"
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
        '{schema:$schema, repo:$repo, pr:$pr, generated_at:$generated_at, rollback_handle:$rollback_handle, changed_files:$changed_files, gate_evidence:$gate_evidence, blast_radius:$blast_radius}' \
        > "$out"
    printf 'DECISION:provenance-written\n'
    printf 'PROVENANCE:%s\n' "$out"
}

main() {
    [ "$#" -gt 0 ] || { usage >&2; exit 2; }
    local sub="$1"; shift
    case "$sub" in
        diff-guard) cmd_diff_guard "$@" ;;
        blast-radius) cmd_blast_radius "$@" ;;
        provenance) cmd_provenance "$@" ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown subcommand: $sub" ;;
    esac
}

main "$@"
