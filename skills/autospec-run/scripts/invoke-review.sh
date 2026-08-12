#!/usr/bin/env bash
# invoke-review.sh — harness-neutral Phase 5.5 broad-review invoker (issue #1433).
#
# Wraps /autospec-review --remediation so Phase 5.5 can reach it from any
# supported harness (Claude Code / Codex CLI / OpenCode) without hardcoding omx.
#
# Usage:
#   bash invoke-review.sh --remediation --since <ISO-DATE> --emit-gaps <FILE> [--outcomes <FILE>]
#
# Exit codes: 0 always (non-blocking per Phase 5.5 semantics).
#   Review ran     — gap file written by the skill.
#   Backend absent — diagnostic gap appended + code_health: emitted to stderr.
#   Invocation err — WARN emitted to stderr; run continues.
#
# Test hooks (env vars):
#   AUTOSPEC_HANDOFF_DISPATCHER_KIND  — override harness detection
#   AUTOSPEC_HARNESS_PROBE_ROOT       — override $HOME for skill-mount probe
#   AUTOSPEC_INVOKE_REVIEW_DRY_RUN    — print resolved command, do not execute

set -eu

# ── Argument parsing ─────────────────────────────────────────────────────────
REMEDIATION=0
SINCE=""
GAPS_FILE=""
OUTCOMES_FILE="${AUTOSPEC_REVIEW_OUTCOMES:-.autospec/review-outcomes.jsonl}"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --remediation) REMEDIATION=1;           shift ;;
        --since)       SINCE="${2:?}";          shift 2 ;;
        --emit-gaps)   GAPS_FILE="${2:?}";      shift 2 ;;
        --outcomes)    OUTCOMES_FILE="${2:?}";  shift 2 ;;
        --help) printf 'Usage: invoke-review.sh --remediation --since <DATE> --emit-gaps <FILE> [--outcomes <FILE>]\n'; exit 0 ;;
        *) printf 'invoke-review: unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done
if [ "$REMEDIATION" -ne 1 ]; then printf 'invoke-review: --remediation required\n' >&2; exit 1; fi
if [ -z "$SINCE" ];           then printf 'invoke-review: --since required\n'       >&2; exit 1; fi
if [ -z "$GAPS_FILE" ];       then printf 'invoke-review: --emit-gaps required\n'   >&2; exit 1; fi

# ── Load harness-detect lib ──────────────────────────────────────────────────
# Installed at $AUTOSPEC_SCRIPTS_DIR/lib/; repo root at scripts/lib/.
_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/autospec-harness-detect.sh"
if [ ! -f "$_LIB" ]; then
    _LIB="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lib/autospec-harness-detect.sh"
fi
if [ -f "$_LIB" ]; then
    # shellcheck source=scripts/lib/autospec-harness-detect.sh
    . "$_LIB"
fi

# ── Emit a LOUD unavailable warning + diagnostic gap (never silent empty gap) ─
_emit_unavailable() {
    local kind="${1:-unknown}"
    printf '\n' >&2
    printf 'invoke-review: WARNING: Phase 5.5 broad review SKIPPED\n' >&2
    printf 'invoke-review: autospec-review backend unavailable (harness=%s)\n' "$kind" >&2
    printf 'invoke-review: This is NOT a clean pass — the broad audit did not run.\n' >&2
    printf 'invoke-review: Install the harness binary or set AUTOSPEC_HANDOFF_DISPATCHER_KIND.\n' >&2
    printf 'code_health:phase55_broad_review_backend_unavailable harness=%s\n' "$kind" >&2

    # Append one diagnostic gap so the file is visibly non-empty.
    # An empty gap file would look like a clean pass to Phase 5.5.
    local dedupe_key
    dedupe_key="phase55-broad-review-unavailable-$(date -u +%Y%m%d)"
    local new_gap
    new_gap="$(printf '{"gap_id":"G0","dimension":"tooling","severity":"high","file":"","line":1,"title":"Phase 5.5 broad review skipped: autospec-review backend unavailable (harness=%s)","body":"The Phase 5.5 broad-review pass did not run because the autospec-review skill could not be invoked (harness=%s). Cross-PR integration gaps may have been missed. Re-run with a supported harness binary on PATH or set AUTOSPEC_HANDOFF_DISPATCHER_KIND.","dedupe_key":"%s"}' \
        "$kind" "$kind" "$dedupe_key")"

    if [ -s "$GAPS_FILE" ]; then
        local merged
        merged="$(jq --argjson g "$new_gap" '. + [$g]' "$GAPS_FILE" 2>/dev/null)" \
            && printf '%s' "$merged" > "$GAPS_FILE" \
            || printf '[%s]\n' "$new_gap" >> "$GAPS_FILE"
    else
        printf '[%s]\n' "$new_gap" > "$GAPS_FILE"
    fi

    local payload canonical digest row
    payload="$(jq -cn --arg run "$SINCE" --arg harness "$kind" \
        '{schema:1,outcome:"review_unavailable",pr:null,phase55_run:$run,reviewer_harness:$harness}')"
    canonical="$(printf '%s' "$payload" | jq -cS '.')"
    if command -v sha256sum >/dev/null 2>&1; then
        digest="$(printf '%s' "$canonical" | sha256sum | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        digest="$(printf '%s' "$canonical" | shasum -a 256 | awk '{print $1}')"
    else
        printf 'invoke-review: WARN: cannot hash review_unavailable outcome\n' >&2
        return 0
    fi
    row="$(printf '%s' "$payload" | jq -c --arg digest "sha256:${digest}" '. + {outcome_digest:$digest}')"
    mkdir -p "$(dirname "$OUTCOMES_FILE")" 2>/dev/null || true
    if [ ! -f "$OUTCOMES_FILE" ] || ! grep -Fq "\"outcome_digest\":\"sha256:${digest}\"" "$OUTCOMES_FILE"; then
        printf '%s\n' "$row" >> "$OUTCOMES_FILE"
    fi
}

# ── Detect harness + resolve dispatcher binary ────────────────────────────────
# NOTE: autospec_harness_resolve_dispatcher calls `exit` on failure (not return),
# so we use autospec_harness_detect + autospec_harness_binary_for + PATH probe
# directly to keep exit-code handling local to this script.
if command -v autospec_harness_detect >/dev/null 2>&1; then
    _KIND="$(autospec_harness_detect)"
else
    _KIND="unknown"
fi

_DISPATCHER=""
if command -v autospec_harness_binary_for >/dev/null 2>&1; then
    _BIN="$(autospec_harness_binary_for "$_KIND" 2>/dev/null)" || _BIN=""
else
    case "$_KIND" in
        claude)   _BIN="claude"   ;;
        codex)    _BIN="codex"    ;;
        opencode) _BIN="opencode" ;;
        *)        _BIN=""         ;;
    esac
fi

if [ -n "$_BIN" ] && command -v "$_BIN" >/dev/null 2>&1; then
    _DISPATCHER="$(command -v "$_BIN")"
    # Reject tmpdir-resident binaries (mirrors autospec-harness-detect.sh safety checks).
    case "$_DISPATCHER" in
        /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*)
            if [ -z "${AUTOSPEC_HANDOFF_DISPATCHER:-}" ]; then
                _DISPATCHER=""
            fi
            ;;
    esac
fi

if [ -z "$_DISPATCHER" ]; then
    _emit_unavailable "$_KIND"
    exit 0
fi

# ── Build and execute the harness invocation ──────────────────────────────────
case "$_KIND" in
    codex)
        # Codex CLI takes the full slash-command as a single string argument.
        set -- "$_DISPATCHER" exec --skip-git-repo-check \
            "/autospec-review --remediation --since ${SINCE} --emit-gaps ${GAPS_FILE}"
        ;;
    *)
        # Claude Code + OpenCode: slash-command and flags as separate argv.
        set -- "$_DISPATCHER" "/autospec-review" \
            "--remediation" "--since" "${SINCE}" "--emit-gaps" "${GAPS_FILE}"
        ;;
esac

if [ -n "${AUTOSPEC_INVOKE_REVIEW_DRY_RUN:-}" ]; then
    printf 'DRY-RUN:'; printf ' %s' "$@"; printf '\n'
    exit 0
fi

if "$@"; then
    exit 0
fi

# Non-zero exit from harness: log a warning, never block the run.
printf 'invoke-review: WARN: /autospec-review exited non-zero on harness=%s\n' "$_KIND" >&2
printf 'code_health:phase55_broad_review_invocation_failed harness=%s\n' "$_KIND" >&2
_emit_unavailable "${_KIND}-invocation-failed"
exit 0
