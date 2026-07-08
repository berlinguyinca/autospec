#!/usr/bin/env bash
set -eu
SCRIPT_NAME="autonomous-premerge-gate"
PR_BRANCH=""
REPO=""
PR_NUMBER=""
MAX_ATTEMPTS="${AUTOSPEC_PREMERGE_MAX_ATTEMPTS:-5}"
NOTIFY_SH=""
CHANGED_FILES=""
GATE_EVIDENCE=""
ROLLBACK_HANDLE=""
PROVENANCE_OUT=""
FENCED_SURFACES=""
QUARANTINE_OUT=""
DRY_RUN=0
LANE="implementer"
MUTATION_BASELINE=""
MUTATION_CURRENT=""
_default_notify_sh() {
    local dir
    dir="$(cd "$(dirname "$0")" && pwd)"
    if [ -x "$dir/../skills/autospec-shared/scripts/notify.sh" ]; then
        printf '%s' "$dir/../skills/autospec-shared/scripts/notify.sh"
    elif command -v notify.sh >/dev/null 2>&1; then
        printf 'notify.sh'
    else
        printf ''
    fi
}
usage() {
    cat <<'EOF'
Usage: autonomous-premerge-gate.sh [OPTIONS]
Options:
  --pr-branch <branch>    Branch to validate (default: current branch).
  --repo <owner/repo>     GitHub repo slug for labelling (default: detected).
  --pr <number>           PR number for labelling (optional).
  --max-attempts <N>      Max fix-and-recheck cycles per stage (default: 5).
  --notify-sh <path>      Path to notify.sh (default: auto-detected).
  --lane <name>           Actor lane for immutable verifier bypass (implementer/verifier).
  --changed-files <file>  Newline-delimited changed files for immutable/blast checks.
  --fenced-surfaces <yml> Config-driven fenced-surface registry.
  --quarantine-out <json> Write async human-review quarantine provenance JSON.
  --gate-evidence <json>  Passing gate evidence JSON to record on merge-ok.
  --mutation-baseline <json> Baseline mutation ledger JSON.
  --mutation-current <json>  Current mutation ledger JSON.
  --rollback-handle <ref> Rollback ref/command handle to record on merge-ok.
  --provenance-out <json> Output path for merge provenance JSON.
  --dry-run               Print what would run without executing.
Environment:
  AUTOSPEC_PREMERGE_MAX_ATTEMPTS  Override max attempts (default 5).
  AUTOSPEC_QA_CMD                 Override the qa command (default: autospec-qa).
  AUTOSPEC_SECAUDIT_CMD           Override the secaudit command (default: autospec-secaudit).
  AUTOSPEC_NOTIFY                 Set to 0 to suppress notifications.
Exit 0 = merge-ok; 1 = block; 2 = halt (code_health); 3 = invocation error.
EOF
}
die() {
    printf '%s: ERROR: %s\n' "$SCRIPT_NAME" "$1" >&2
    exit 3
}
info() {
    printf '[%s] %s\n' "$SCRIPT_NAME" "$*" >&2
}
while [ $# -gt 0 ]; do
    case "$1" in
        --pr-branch)   PR_BRANCH="${2:-}"; shift 2 ;;
        --repo)        REPO="${2:-}"; shift 2 ;;
        --pr)          PR_NUMBER="${2:-}"; shift 2 ;;
        --max-attempts) MAX_ATTEMPTS="${2:-5}"; shift 2 ;;
        --notify-sh)   NOTIFY_SH="${2:-}"; shift 2 ;;
        --lane)        LANE="${2:-}"; shift 2 ;;
        --changed-files) CHANGED_FILES="${2:-}"; shift 2 ;;
        --gate-evidence) GATE_EVIDENCE="${2:-}"; shift 2 ;;
        --mutation-baseline) MUTATION_BASELINE="${2:-}"; shift 2 ;;
        --mutation-current) MUTATION_CURRENT="${2:-}"; shift 2 ;;
        --rollback-handle) ROLLBACK_HANDLE="${2:-}"; shift 2 ;;
        --provenance-out) PROVENANCE_OUT="${2:-}"; shift 2 ;;
        --fenced-surfaces) FENCED_SURFACES="${2:-}"; shift 2 ;;
        --quarantine-out) QUARANTINE_OUT="${2:-}"; shift 2 ;;
        --dry-run)     DRY_RUN=1; shift ;;
        --help|-h)     usage; exit 0 ;;
        *) printf '%s: unknown argument: %s\n' "$SCRIPT_NAME" "$1" >&2
           usage >&2
           exit 3 ;;
    esac
done
if [ -z "$NOTIFY_SH" ]; then
    NOTIFY_SH="$(_default_notify_sh)"
fi
_notify() {
    local title="$1" body="$2"
    if [ -n "$NOTIFY_SH" ] && [ -x "$NOTIFY_SH" ]; then
        "$NOTIFY_SH" "$title" "$body" || true
    elif [ -n "$NOTIFY_SH" ] && command -v "$NOTIFY_SH" >/dev/null 2>&1; then
        "$NOTIFY_SH" "$title" "$body" || true
    else
        info "notify: $title — $body"
    fi
}
if [ -z "$PR_BRANCH" ]; then
    PR_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
fi
if [ -z "$PR_BRANCH" ] || [ "$PR_BRANCH" = "HEAD" ]; then
    die "could not determine PR branch; pass --pr-branch"
fi
case "$LANE" in
    implementer|verifier) ;;
    *) die "--lane must be implementer or verifier" ;;
esac
info "Validating branch: $PR_BRANCH"
info "Actor lane: $LANE"
info "Max fix-and-recheck attempts: $MAX_ATTEMPTS"
QA_CMD="${AUTOSPEC_QA_CMD:-autospec-qa}"
SECAUDIT_CMD="${AUTOSPEC_SECAUDIT_CMD:-autospec-secaudit}"
_qa_skill_present() {
    if [ -n "${AUTOSPEC_QA_PRESENT_OVERRIDE:-}" ]; then
        if [ "$AUTOSPEC_QA_PRESENT_OVERRIDE" = "true" ]; then
            return 0
        else
            return 1
        fi
    fi
    command -v "$QA_CMD" >/dev/null 2>&1
}
_secaudit_skill_present() {
    if [ -n "${AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE:-}" ]; then
        if [ "$AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE" = "true" ]; then
            return 0
        else
            return 1
        fi
    fi
    command -v "$SECAUDIT_CMD" >/dev/null 2>&1
}
if ! _qa_skill_present; then
    printf 'halt code_health:qa_skill_missing\n'
    info "HALT: configured scan skill '$QA_CMD' is not installed or not in PATH."
    info "Install autospec-qa before re-running the pre-merge gate."
    exit 2
fi
info "Scan skill '$QA_CMD' is present."
if ! _secaudit_skill_present; then
    printf 'halt code_health:secaudit_skill_missing\n'
    info "HALT: configured scan skill '$SECAUDIT_CMD' is not installed or not in PATH."
    info "Install autospec-secaudit before re-running the pre-merge gate."
    exit 2
fi
info "Scan skill '$SECAUDIT_CMD' is present."
_count_blocking_findings() {
    grep -ciE \
        '(severity[[:space:]]*:[[:space:]]*(high|severe|critical|medium)|\[(high|severe|critical|medium)\]|^(high|severe|critical|medium)[[:space:]])' \
        || true
}
_has_blocking_findings() {
    local scan_output="$1"
    local count
    count=$(printf '%s' "$scan_output" | _count_blocking_findings)
    if [ "${count:-0}" -gt 0 ]; then
        return 0  # has blocking findings
    else
        return 1  # no blocking findings
    fi
}
_log_low_findings() {
    local scan_output="$1" label="$2"
    local low_findings
    low_findings=$(printf '%s' "$scan_output" | \
        grep -iE '(severity[[:space:]]*:[[:space:]]*(low|info)|\[(low|info)\]|^(low|info)[[:space:]])' || true)
    if [ -n "$low_findings" ]; then
        info "$label low/info findings (non-blocking, recorded):"
        printf '%s\n' "$low_findings" >&2
    fi
}
_run_qa() {
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] would run: $QA_CMD --branch $PR_BRANCH"
        printf ''
        return 0
    fi
    "$QA_CMD" --branch "$PR_BRANCH" 2>&1 || true
}
_run_secaudit() {
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] would run: $SECAUDIT_CMD --branch $PR_BRANCH"
        printf ''
        return 0
    fi
    "$SECAUDIT_CMD" --branch "$PR_BRANCH" 2>&1 || true
}
_dispatch_fix() {
    local attempt="$1" scan_output="$2" stage="$3"
    local fix_cmd="${AUTOSPEC_FIX_CMD:-}"
    info "Dispatching fix for $stage blocking findings (attempt $attempt/$MAX_ATTEMPTS) ..."
    if [ -n "$fix_cmd" ]; then
        if [ "$DRY_RUN" -eq 1 ]; then
            info "[dry-run] would run: $fix_cmd --branch $PR_BRANCH"
        else
            printf '%s' "$scan_output" | "$fix_cmd" --branch "$PR_BRANCH" || true
        fi
    else
        info "No AUTOSPEC_FIX_CMD set; recording findings for operator review."
    fi
}
_apply_needs_human_label() {
    if [ -z "$REPO" ] && [ -z "$PR_NUMBER" ]; then
        info "No --repo/--pr supplied; skipping label application."
        return
    fi
    local label="autospec:needs-human"
    if [ -n "$REPO" ] && [ -n "$PR_NUMBER" ]; then
        if [ "$DRY_RUN" -eq 1 ]; then
            info "[dry-run] would apply label '$label' to PR #$PR_NUMBER in $REPO"
        else
            gh pr edit "$PR_NUMBER" --repo "$REPO" --add-label "$label" 2>&1 || \
                info "WARN: could not apply label '$label' (continuing)"
        fi
    elif [ -n "$PR_NUMBER" ]; then
        if [ "$DRY_RUN" -eq 1 ]; then
            info "[dry-run] would apply label '$label' to PR #$PR_NUMBER"
        else
            gh pr edit "$PR_NUMBER" --add-label "$label" 2>&1 || \
                info "WARN: could not apply label '$label' (continuing)"
        fi
    fi
}
_guardrails_sh() {
    local dir
    dir="$(cd "$(dirname "$0")" && pwd)"
    printf '%s/autonomous-guardrails.sh' "$dir"
}
_apply_guardrail_block_label() {
    _apply_needs_human_label
}
_write_quarantine_record() {
    local classification_json="$1" out="$2"
    [ -n "$out" ] || return 0
    local generated_at changed_json
    generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ -n "$CHANGED_FILES" ] && [ -f "$CHANGED_FILES" ]; then
        changed_json="$(sed '/^[[:space:]]*$/d; s#^\./##' "$CHANGED_FILES" \
            | jq -R -s 'split("\n") | map(select(length > 0))')"
    else
        changed_json='[]'
    fi
    mkdir -p "$(dirname "$out")"
    jq -n \
        --arg schema 'autospec.autonomous.quarantine.v1' \
        --arg queue 'human-review' \
        --arg repo "$REPO" \
        --arg pr "${PR_NUMBER:-0}" \
        --arg branch "$PR_BRANCH" \
        --arg generated_at "$generated_at" \
        --arg reason 'fenced_surface' \
        --argjson changed_files "$changed_json" \
        --argjson classification "$classification_json" \
        '{schema:$schema, queue:$queue, repo:$repo, pr:($pr|tonumber), branch:$branch, generated_at:$generated_at, reason:$reason, changed_files:$changed_files, classification:$classification}' \
        > "$out"
}
if [ -n "$CHANGED_FILES" ]; then
    GUARDRAILS_SH="$(_guardrails_sh)"
    if ! guard_output="$(bash "$GUARDRAILS_SH" diff-guard --lane "$LANE" --changed-files "$CHANGED_FILES" 2>&1)"; then
        printf "%s\n" "$guard_output"
        _apply_guardrail_block_label
        printf "block immutable_verifier_modified\n"
        exit 1
    fi
    blast_status=0
    if [ -n "$FENCED_SURFACES" ]; then
        blast_output="$(bash "$GUARDRAILS_SH" blast-radius --changed-files "$CHANGED_FILES" --fenced-surfaces "$FENCED_SURFACES" --json 2>&1)" || blast_status=$?
    else
        blast_output="$(bash "$GUARDRAILS_SH" blast-radius --changed-files "$CHANGED_FILES" --json 2>&1)" || blast_status=$?
    fi
    if [ "$blast_status" -ne 0 ]; then
        printf "%s\n" "$blast_output"
        _apply_guardrail_block_label
        _write_quarantine_record "$blast_output" "$QUARANTINE_OUT"
        _notify "autospec: quarantined" \
            "Pre-merge gate quarantined $PR_BRANCH for fenced-surface blast radius; continuing autonomous loop."
        printf "quarantine fenced_surface\n"
        exit 0
    fi
fi
if [ -n "$MUTATION_BASELINE" ] || [ -n "$MUTATION_CURRENT" ]; then
    [ -n "$MUTATION_BASELINE" ] || die "--mutation-current requires --mutation-baseline"
    [ -n "$MUTATION_CURRENT" ] || die "--mutation-baseline requires --mutation-current"
    GUARDRAILS_SH="$(_guardrails_sh)"
    if ! mutation_output="$(bash "$GUARDRAILS_SH" mutation-guard --baseline "$MUTATION_BASELINE" --current "$MUTATION_CURRENT" 2>&1)"; then
        printf "%s\n" "$mutation_output"
        _apply_guardrail_block_label
        printf "block mutation_score_regression\n"
        exit 1
    fi
fi
attempt=1
while true; do
    info "=== QA run (attempt $attempt/$MAX_ATTEMPTS) ==="
    qa_output=""
    qa_output="$(_run_qa)"
    _log_low_findings "$qa_output" "QA"
    if _has_blocking_findings "$qa_output"; then
        if [ "$attempt" -ge "$MAX_ATTEMPTS" ]; then
            info "Max attempts ($MAX_ATTEMPTS) reached with QA blocking findings still present."
            _apply_needs_human_label
            _notify "autospec: needs-human" \
                "Pre-merge gate exhausted $MAX_ATTEMPTS QA attempts on $PR_BRANCH; blocking findings remain."
            printf 'block retries_exhausted\n'
            exit 1
        fi
        info "Blocking QA findings detected; dispatching fix (attempt $attempt/$MAX_ATTEMPTS)."
        _dispatch_fix "$attempt" "$qa_output" "qa"
        attempt=$((attempt + 1))
        continue
    fi
    info "QA stage clear on attempt $attempt."
    break
done
secaudit_attempt=1
while true; do
    info "=== secaudit run (attempt $secaudit_attempt/$MAX_ATTEMPTS) ==="
    secaudit_output=""
    secaudit_output="$(_run_secaudit)"
    _log_low_findings "$secaudit_output" "secaudit"
    if _has_blocking_findings "$secaudit_output"; then
        if [ "$secaudit_attempt" -ge "$MAX_ATTEMPTS" ]; then
            info "Max attempts ($MAX_ATTEMPTS) reached with secaudit blocking findings still present."
            _apply_needs_human_label
            _notify "autospec: needs-human" \
                "Pre-merge gate exhausted $MAX_ATTEMPTS secaudit attempts on $PR_BRANCH; blocking findings remain."
            printf 'block retries_exhausted\n'
            exit 1
        fi
        info "Blocking secaudit findings detected; dispatching fix (attempt $secaudit_attempt/$MAX_ATTEMPTS)."
        _dispatch_fix "$secaudit_attempt" "$secaudit_output" "secaudit"
        secaudit_attempt=$((secaudit_attempt + 1))
        continue
    fi
    info "No blocking findings on QA+secaudit. Gate passes."
    if [ -n "$PROVENANCE_OUT" ]; then
        if [ -z "$CHANGED_FILES" ] || [ -z "$GATE_EVIDENCE" ] || [ -z "$ROLLBACK_HANDLE" ] || [ -z "$REPO" ] || [ -z "$PR_NUMBER" ]; then
            die "--provenance-out requires --changed-files, --gate-evidence, --rollback-handle, --repo, and --pr"
        fi
        bash "$(_guardrails_sh)" provenance \
            --repo "$REPO" \
            --pr "$PR_NUMBER" \
            --changed-files "$CHANGED_FILES" \
            --gate-evidence "$GATE_EVIDENCE" \
            --rollback-handle "$ROLLBACK_HANDLE" \
            --out "$PROVENANCE_OUT" >&2
    fi
    printf 'merge-ok\n'
    exit 0
done
