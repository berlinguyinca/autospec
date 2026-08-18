#!/usr/bin/env bash
# scripts/calibrate-profile.sh — qualify a profile before trusting it with real work.
#
# R11. Cold-start exploration (route-decide.sh) learns on live issues; this learns
# on issues whose correct outcome is already known, so a profile can be qualified
# without risking anything. It replays K previously-merged issues against a
# candidate profile in a throwaway worktree, scores each attempt with the repo's
# OWN gate, and writes the results into the routing ledger as ordinary rows — so
# the same effective-cost formula consumes calibration and live evidence alike.
#
# "This profile qualified for zero tiers" is a legitimate, expected result and is
# reported as a clean exit, not retried into submission. On a host whose GPU is
# unusable, zero is the CORRECT answer, and a harness that kept trying until it
# got a pass would be manufacturing evidence.
#
# The verdict is cached per hardware fingerprint (from discover-model-supply.sh):
# re-running on unchanged hardware is a no-op, and swapping a GPU invalidates it.
#
# §32 makes qualification PER ROLE: "qualified for implementation and docs, not
# qualified for planning and review" is one calibration result, not four runs of
# a pass/fail tool. --role scopes the verdict to a single role and writes it to
# its own file, so discover-model-supply.sh can lift exactly that role to §8
# `calibrated` and leave every other role at `advertised`.
#
# §33 bounds cold-start exploration with --exploration-budget, and forbids it
# outright for the security and independent-review roles: collecting statistics
# is never a reason to let an unqualified model near the gate that would catch
# its own mistakes.
#
# Usage:
#   calibrate-profile.sh --profile|--calibrate <name> [--model <tag>]
#                        [--role <role>] [--exploration-budget N]
#                        [--issues <n1,n2,...>] [--count K]
#                        [--gate-cmd "<command>"] [--repo <owner/name>]
#                        [--dry-run] [--force] [--json]
#
# Exit codes:
#   0  calibration ran (or a cached verdict was reused); read `qualified`
#   1  bad arguments, including a role outside the 14-role vocabulary
#   3  cannot calibrate — no capability document, no local dispatch, or no
#      replayable issues. Distinct from "ran and failed": nothing was measured.
#   4  exploration refused — see reason= (§33)
#
# Environment:
#   AUTOSPEC_CALIBRATION_DIR   verdict cache (default ~/.autospec/calibration)
#   AUTOSPEC_CALIBRATION_GATE  default gate command
#   AUTOSPEC_ROUTING_LEDGER    forwarded to routing-ledger.sh

set -u

PROFILE=""; MODEL=""; ISSUES=""; COUNT=5
ROLE=""; EXPLORATION_BUDGET=""
GATE_CMD="${AUTOSPEC_CALIBRATION_GATE:-}"
REPO=""; DRY_RUN=0; FORCE=0; JSON=0
CACHE_DIR="${AUTOSPEC_CALIBRATION_DIR:-$HOME/.autospec/calibration}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

# The 14 snake_case roles (§3, ADR 0001 D2) — kept in step with
# scripts/lib/model-capability-evidence.sh.
ROLE_VOCABULARY="orchestrator planner architect test_planner implementer
code_reviewer test_reviewer qa_verifier documentation_writer
documentation_reviewer ui_ux_reviewer security_reviewer researcher advisor"

# §33: never explore into security or independent review. A model that has not
# earned a role cannot be handed the very role that would judge its own output,
# and "we needed the statistics" is not an exception.
EXPLORATION_FORBIDDEN_ROLES="security_reviewer code_reviewer test_reviewer
documentation_reviewer ui_ux_reviewer qa_verifier"

_die() { printf 'calibrate-profile: %s\n' "$1" >&2; exit "${2:-1}"; }
_refuse() { printf 'calibrate-profile: %s\n' "$1" >&2; exit 3; }
# _forbid <message> <reason> — an exploration refusal. The reason is a stable
# code callers can branch on; the specific role stays in the human message so
# `reason` never becomes a free-text field.
_forbid() { printf 'calibrate-profile: %s reason=%s\n' "$1" "$2" >&2; exit 4; }

# _in_list <needle> <whitespace-separated list>
_in_list() {
    for _item in $2; do
        if [ "$_item" = "$1" ]; then return 0; fi
    done
    return 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --profile|--calibrate)  PROFILE="${2:-}"; shift 2 ;;
        --model)    MODEL="${2:-}"; shift 2 ;;
        --role)     ROLE="${2:-}"; shift 2 ;;
        --exploration-budget) EXPLORATION_BUDGET="${2:-}"; shift 2 ;;
        --issues)   ISSUES="${2:-}"; shift 2 ;;
        --count)    COUNT="${2:-}"; shift 2 ;;
        --gate-cmd) GATE_CMD="${2:-}"; shift 2 ;;
        --repo)     REPO="${2:-}"; shift 2 ;;
        --dry-run)  DRY_RUN=1; shift ;;
        --force)    FORCE=1; shift ;;
        --json)     JSON=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$PROFILE" ]; then _die '--profile is required'; fi
case "$COUNT" in ''|*[!0-9]*) _die "--count must be an integer: $COUNT" ;; esac
if [ "$COUNT" -lt 1 ] 2>/dev/null; then _die '--count must be >= 1'; fi
if ! command -v jq >/dev/null 2>&1; then _refuse 'jq is required'; fi

if [ -n "$ROLE" ]; then
    if ! _in_list "$ROLE" "$ROLE_VOCABULARY"; then
        _die "unknown role: $ROLE (expected one of: $(printf '%s' "$ROLE_VOCABULARY" | tr '\n' ' '))"
    fi
fi

# §33 cold start. The budget caps how many replays exploration may spend, and
# gates WHICH roles exploration may touch at all. Both checks run before any
# dispatch: a refusal after the first replay would already have spent the risk.
if [ -n "$EXPLORATION_BUDGET" ]; then
    case "$EXPLORATION_BUDGET" in
        ''|*[!0-9]*) _die "--exploration-budget must be a non-negative integer: $EXPLORATION_BUDGET" ;;
    esac
    if [ -z "$ROLE" ]; then
        _die '--exploration-budget requires --role (exploration is gated per role)'
    fi
    if _in_list "$ROLE" "$EXPLORATION_FORBIDDEN_ROLES"; then
        _forbid "exploration may never qualify role $ROLE" role_forbidden
    fi
    if [ "$EXPLORATION_BUDGET" -eq 0 ]; then
        _forbid "exploration budget is 0; no replay may be spent on role $ROLE" exploration_budget_exhausted
    fi
fi

# ── hardware fingerprint (verdicts are only valid for the hardware measured) ───
FP="unknown"
_probe="$SCRIPT_DIR/discover-model-supply.sh"
if [ -f "$_probe" ]; then
    FP="$(bash "$_probe" --fingerprint 2>/dev/null || printf 'unknown')"
fi
if [ -z "$FP" ]; then FP="unknown"; fi

# A role-scoped verdict gets its own file. The profile-level path is left exactly
# where it was so an existing cache — and every caller that reads it — keeps
# working; the two never collide because a role name cannot be empty.
VERDICT_FILE="$CACHE_DIR/${PROFILE}.${FP}.json"
VERDICT_ROLE="any"
if [ -n "$ROLE" ]; then
    VERDICT_FILE="$CACHE_DIR/${PROFILE}.${FP}.${ROLE}.json"
    VERDICT_ROLE="$ROLE"
fi

_report() {
    if [ "$JSON" -eq 1 ]; then
        cat "$VERDICT_FILE"
    else
        jq -r '"profile=\(.profile) role=\(.role // "any") qualified=\(.qualified) passed=\(.passed)/\(.attempted) fingerprint=\(.fingerprint)"' \
            "$VERDICT_FILE" 2>/dev/null || printf 'profile=%s (no verdict)\n' "$PROFILE"
    fi
}

if [ "$FORCE" -eq 0 ] && [ -f "$VERDICT_FILE" ]; then
    # Unchanged hardware means the previous measurement still stands.
    _report
    exit 0
fi

# ── resolve the model id to dispatch ──────────────────────────────────────────
if [ -z "$MODEL" ]; then
    _sel=""
    for _c in "$SCRIPT_DIR/select-model-profile.sh" \
              "$SCRIPT_DIR/../skills/autospec-run/scripts/select-model-profile.sh" \
              "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/select-model-profile.sh"; do
        if [ -f "$_c" ]; then _sel="$_c"; break; fi
    done
    if [ -n "$_sel" ]; then
        MODEL="$(AUTOSPEC_TIER_B_PROFILE="$PROFILE" bash "$_sel" --labels "" --print-model 2>/dev/null || printf '')"
    fi
fi
if [ -z "$MODEL" ]; then
    _refuse "cannot resolve a model id for profile $PROFILE (no model: key?)"
fi

# ── the replay set: issues whose correct outcome is already known ─────────────
if [ -z "$ISSUES" ]; then
    if ! command -v gh >/dev/null 2>&1; then
        _refuse 'no --issues given and gh is unavailable; cannot build a replay set'
    fi
    _repo_args=""
    if [ -n "$REPO" ]; then _repo_args="--repo $REPO"; fi
    # shellcheck disable=SC2086
    ISSUES="$(gh pr list $_repo_args --state merged --limit "$COUNT" \
                --json number -q 'map(.number)|join(",")' 2>/dev/null || printf '')"
fi
if [ -z "$ISSUES" ]; then
    _refuse 'no replayable merged issues found'
fi

# §33: the budget is a hard ceiling on replays, applied AFTER the set is built so
# an operator's explicit --issues list is truncated rather than silently ignored.
if [ -n "$EXPLORATION_BUDGET" ]; then
    _kept=""; _spent=0
    _b_ifs="$IFS"; IFS=','
    for _n in $ISSUES; do
        IFS="$_b_ifs"
        if [ "$_spent" -ge "$EXPLORATION_BUDGET" ]; then break; fi
        if [ -z "$_kept" ]; then _kept="$_n"; else _kept="$_kept,$_n"; fi
        _spent=$((_spent + 1))
        IFS=','
    done
    IFS="$_b_ifs"
    ISSUES="$_kept"
    if [ -z "$ISSUES" ]; then
        _forbid "exploration budget left no replay for role $ROLE" exploration_budget_exhausted
    fi
fi

# ── gate: the repo's own definition of correct ─────────────────────────────────
if [ -z "$GATE_CMD" ]; then
    if command -v autospec >/dev/null 2>&1; then
        GATE_CMD="autospec validate"
    else
        _refuse 'no --gate-cmd and no autospec binary; refusing to score with no gate'
    fi
fi

if [ "$DRY_RUN" -eq 1 ]; then
    printf 'would calibrate profile=%s model=%s role=%s\n' "$PROFILE" "$MODEL" "$VERDICT_ROLE"
    printf 'replay issues: %s\n' "$ISSUES"
    printf 'gate: %s\n' "$GATE_CMD"
    printf 'verdict file: %s\n' "$VERDICT_FILE"
    exit 0
fi

# ── replay ────────────────────────────────────────────────────────────────────
attempted=0
passed=0
_old_ifs="$IFS"; IFS=','
for _issue in $ISSUES; do
    IFS="$_old_ifs"
    attempted=$((attempted + 1))
    _wt="$(mktemp -d "${TMPDIR:-/tmp}/autospec-calib-XXXXXX")"
    _prompt="$_wt/prompt.txt"
    printf 'Replay of already-merged issue #%s for profile calibration.\n' "$_issue" > "$_prompt"

    _start="$(date -u +%s)"
    _rc=0
    if [ -f "$SCRIPT_DIR/local-dispatch.sh" ]; then
        bash "$SCRIPT_DIR/local-dispatch.sh" --model "$MODEL" --prompt-file "$_prompt" \
            --cwd "$_wt" >/dev/null 2>&1 || _rc=$?
    else
        _rc=3
    fi
    _elapsed_ms=$(( ($(date -u +%s) - _start) * 1000 ))

    _outcome="qa_failed"
    if [ "$_rc" -eq 0 ]; then
        if ( cd "$_wt" && eval "$GATE_CMD" >/dev/null 2>&1 ); then
            _outcome="merged_clean"
            passed=$((passed + 1))
        fi
    elif [ "$_rc" -eq 4 ]; then
        _outcome="abandoned"
    fi

    # Record as an ordinary ledger row so calibration and live evidence share one
    # formula. dispatch_id is namespaced so a replay is never mistaken for a
    # real dispatch of that issue.
    if [ -f "$SCRIPT_DIR/routing-ledger.sh" ]; then
        _rec="$(jq -nc --arg id "calib-$PROFILE-$VERDICT_ROLE-$FP-$_issue" --arg p "$PROFILE" --arg m "$MODEL" \
            --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg oc "$_outcome" \
            --argjson issue "$_issue" --argjson ms "$_elapsed_ms" \
            '{dispatch_id:$id, ts:$ts, dispatch_kind:"implementer", profile:$p, model:$m,
              harness:"codex-oss", issue:$issue, cell_ctx:"32k", cell_reasoning:"shallow",
              input_tokens:0, output_tokens:0, cached_tokens:0, wall_clock_ms:$ms,
              retries:0, escalated:false, outcome:$oc, reason:"calibration replay"}')"
        bash "$SCRIPT_DIR/routing-ledger.sh" --append "$_rec" >/dev/null 2>&1 || true
    fi
    rm -rf "$_wt"
    IFS=','
done
IFS="$_old_ifs"

# ── verdict ───────────────────────────────────────────────────────────────────
# Qualification needs a majority of replays to clear the repo's own gate. Zero is
# a valid answer and is written down as such.
qualified="false"
if [ "$attempted" -gt 0 ] && [ $((passed * 2)) -gt "$attempted" ]; then
    qualified="true"
fi

if [ ! -d "$CACHE_DIR" ]; then mkdir -p "$CACHE_DIR"; fi
_budget_json="null"
if [ -n "$EXPLORATION_BUDGET" ]; then _budget_json="$EXPLORATION_BUDGET"; fi

jq -n --arg p "$PROFILE" --arg m "$MODEL" --arg fp "$FP" --arg role "$VERDICT_ROLE" \
      --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --argjson attempted "$attempted" --argjson passed "$passed" \
      --argjson qualified "$qualified" --argjson budget "$_budget_json" \
      '{profile:$p, model:$m, role:$role, fingerprint:$fp, calibrated_at:$ts,
        attempted:$attempted, passed:$passed, qualified:$qualified,
        exploration_budget:$budget}' > "$VERDICT_FILE"

_report
exit 0
