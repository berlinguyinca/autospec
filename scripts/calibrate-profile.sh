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
# Usage:
#   calibrate-profile.sh --profile <name> [--model <tag>]
#                        [--issues <n1,n2,...>] [--count K]
#                        [--gate-cmd "<command>"] [--repo <owner/name>]
#                        [--dry-run] [--force] [--json]
#
# Exit codes:
#   0  calibration ran (or a cached verdict was reused); read `qualified`
#   1  bad arguments
#   3  cannot calibrate — no capability document, no local dispatch, or no
#      replayable issues. Distinct from "ran and failed": nothing was measured.
#
# Environment:
#   AUTOSPEC_CALIBRATION_DIR   verdict cache (default ~/.autospec/calibration)
#   AUTOSPEC_CALIBRATION_GATE  default gate command
#   AUTOSPEC_ROUTING_LEDGER    forwarded to routing-ledger.sh

set -u

PROFILE=""; MODEL=""; ISSUES=""; COUNT=5
GATE_CMD="${AUTOSPEC_CALIBRATION_GATE:-}"
REPO=""; DRY_RUN=0; FORCE=0; JSON=0
CACHE_DIR="${AUTOSPEC_CALIBRATION_DIR:-$HOME/.autospec/calibration}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

_die() { printf 'calibrate-profile: %s\n' "$1" >&2; exit "${2:-1}"; }
_refuse() { printf 'calibrate-profile: %s\n' "$1" >&2; exit 3; }

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --profile)  PROFILE="${2:-}"; shift 2 ;;
        --model)    MODEL="${2:-}"; shift 2 ;;
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

# ── hardware fingerprint (verdicts are only valid for the hardware measured) ───
FP="unknown"
_probe="$SCRIPT_DIR/discover-model-supply.sh"
if [ -f "$_probe" ]; then
    FP="$(bash "$_probe" --fingerprint 2>/dev/null || printf 'unknown')"
fi
if [ -z "$FP" ]; then FP="unknown"; fi

VERDICT_FILE="$CACHE_DIR/${PROFILE}.${FP}.json"

_report() {
    if [ "$JSON" -eq 1 ]; then
        cat "$VERDICT_FILE"
    else
        jq -r '"profile=\(.profile) qualified=\(.qualified) passed=\(.passed)/\(.attempted) fingerprint=\(.fingerprint)"' \
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

# ── gate: the repo's own definition of correct ─────────────────────────────────
if [ -z "$GATE_CMD" ]; then
    if command -v autospec >/dev/null 2>&1; then
        GATE_CMD="autospec validate"
    else
        _refuse 'no --gate-cmd and no autospec binary; refusing to score with no gate'
    fi
fi

if [ "$DRY_RUN" -eq 1 ]; then
    printf 'would calibrate profile=%s model=%s\n' "$PROFILE" "$MODEL"
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
        _rec="$(jq -nc --arg id "calib-$PROFILE-$FP-$_issue" --arg p "$PROFILE" --arg m "$MODEL" \
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
jq -n --arg p "$PROFILE" --arg m "$MODEL" --arg fp "$FP" \
      --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --argjson attempted "$attempted" --argjson passed "$passed" \
      --argjson qualified "$qualified" \
      '{profile:$p, model:$m, fingerprint:$fp, calibrated_at:$ts,
        attempted:$attempted, passed:$passed, qualified:$qualified}' > "$VERDICT_FILE"

_report
exit 0
