#!/usr/bin/env bash
# scripts/local-dispatch.sh — run a dispatch on a LOCAL model, fail-closed.
#
# The executor half of the routing design (R1). Until this existed, a local
# profile could be selected but never executed: Claude Code's `Agent(model:)`
# accepts Claude tiers only, so a local model is unreachable from that harness.
#
# Codex CLI is the executor rather than a bespoke HTTP client because it already
# ships native local-model support (`--oss --local-provider ollama|lmstudio`) and
# autospec already depends on it for peer review. A hand-rolled :11434 client
# would be new surface duplicating a maintained upstream path.
#
# Three fail-closed preconditions, checked in order. Any of them means "do not
# dispatch locally" and exits 3 so the caller keeps its cloud tier:
#
#   1. Codex CLI must be present AND advertise --oss. An older Codex silently
#      ignoring the flag would run the dispatch against a PAID cloud model while
#      the caller believed it was local — the most expensive possible failure.
#   2. The capability probe must report the model dispatch_recommended. A model on
#      a host with no usable accelerator runs on CPU at 10-30x the latency; that
#      is a throughput regression dressed as a saving. Pass
#      --skip-capability-check only when the caller has already gated on it.
#   3. A wall-clock ceiling is always applied (default 600s). An unbounded local
#      dispatch can occupy the single GPU for the rest of the run.
#
# Local GPU is capacity-1: two concurrent dispatches to one runtime thrash into
# swap and both blow their ceiling. A host-scoped lock serializes them.
#
# Usage:
#   local-dispatch.sh --model <tag> --prompt-file <path>
#                     [--provider ollama|lmstudio] [--cwd <dir>]
#                     [--timeout-secs N] [--skip-capability-check] [--dry-run]
#
# Exit codes:
#   0  dispatch completed (stdout is the executor's output)
#   1  bad arguments
#   3  precondition failed — caller MUST fall back to its cloud tier
#   4  dispatch exceeded the wall-clock ceiling
#   >4 the executor's own non-zero status
#
# Environment:
#   AUTOSPEC_LOCAL_PROVIDER        default provider (ollama)
#   AUTOSPEC_LOCAL_TIMEOUT_SECS    default ceiling (600)
#   AUTOSPEC_MODEL_CAPABILITY      probe document path
#   AUTOSPEC_LOCAL_LOCK_DIR        lock directory (default ~/.autospec/locks)

set -u

MODEL=""
PROMPT_FILE=""
PROVIDER="${AUTOSPEC_LOCAL_PROVIDER:-ollama}"
WORKDIR="."
TIMEOUT_SECS="${AUTOSPEC_LOCAL_TIMEOUT_SECS:-600}"
SKIP_CAP=0
DRY_RUN=0
CAPABILITY="${AUTOSPEC_MODEL_CAPABILITY:-$HOME/.autospec/model-capability.json}"
LOCK_DIR="${AUTOSPEC_LOCAL_LOCK_DIR:-$HOME/.autospec/locks}"

_die() { printf 'local-dispatch: %s\n' "$1" >&2; exit "${2:-1}"; }
_refuse() { printf 'local-dispatch: %s\n' "$1" >&2; exit 3; }

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --model)        MODEL="${2:-}"; shift 2 ;;
        --prompt-file)  PROMPT_FILE="${2:-}"; shift 2 ;;
        --provider)     PROVIDER="${2:-}"; shift 2 ;;
        --cwd)          WORKDIR="${2:-}"; shift 2 ;;
        --timeout-secs) TIMEOUT_SECS="${2:-}"; shift 2 ;;
        --capability-file) CAPABILITY="${2:-}"; shift 2 ;;
        --skip-capability-check) SKIP_CAP=1; shift ;;
        --dry-run)      DRY_RUN=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$MODEL" ]; then _die '--model is required'; fi
if [ -z "$PROMPT_FILE" ]; then _die '--prompt-file is required'; fi
if [ ! -f "$PROMPT_FILE" ]; then _die "prompt file not found: $PROMPT_FILE"; fi
case "$PROVIDER" in
    ollama|lmstudio) ;;
    *) _die "unsupported provider: $PROVIDER (ollama|lmstudio)" ;;
esac
case "$TIMEOUT_SECS" in
    ''|*[!0-9]*) _die "--timeout-secs must be an integer: $TIMEOUT_SECS" ;;
esac

# ── precondition 1: an executor that really supports local models ─────────────
if ! command -v codex >/dev/null 2>&1; then
    _refuse 'codex CLI not found; local dispatch unavailable'
fi
# Probe the advertised flag rather than trusting a version string: an older Codex
# would ignore --oss and quietly bill a cloud model.
if ! codex --help 2>&1 | grep -q -- '--oss'; then
    _refuse 'codex CLI does not advertise --oss; refusing to risk a paid dispatch'
fi

# ── precondition 2: the host can actually run this model ───────────────────────
if [ "$SKIP_CAP" -eq 0 ]; then
    if [ ! -f "$CAPABILITY" ]; then
        _refuse "no capability document at $CAPABILITY; run discover-model-supply.sh first"
    fi
    if ! command -v jq >/dev/null 2>&1; then
        _refuse 'jq is required to read the capability document'
    fi
    _ok="$(jq -r --arg m "$MODEL" '
        (.local_models // []) | map(select(.model == $m)) | first
        | if . == null then "absent" elif .dispatch_recommended then "yes" else "no" end' \
        "$CAPABILITY" 2>/dev/null || printf 'absent')"
    case "$_ok" in
        yes) ;;
        no)  _refuse "model $MODEL is present but not dispatch_recommended ($(jq -r '.accelerator.reason // "unknown"' "$CAPABILITY" 2>/dev/null))" ;;
        *)   _refuse "model $MODEL not found in the capability document" ;;
    esac
fi

# ── precondition 3: bounded, serialized execution ─────────────────────────────
if ! command -v timeout >/dev/null 2>&1; then
    _refuse 'timeout(1) not found; refusing an unbounded local dispatch'
fi

CODEX_ARGS="exec --oss --local-provider $PROVIDER --model $MODEL"

if [ "$DRY_RUN" -eq 1 ]; then
    printf 'codex %s\n' "$CODEX_ARGS"
    printf 'timeout_secs=%s cwd=%s prompt_file=%s\n' "$TIMEOUT_SECS" "$WORKDIR" "$PROMPT_FILE"
    exit 0
fi

if [ ! -d "$LOCK_DIR" ]; then mkdir -p "$LOCK_DIR"; fi
LOCK="$LOCK_DIR/local-model.lock"

_run_dispatch() {
    # shellcheck disable=SC2086
    timeout --preserve-status "$TIMEOUT_SECS" \
        codex $CODEX_ARGS --cd "$WORKDIR" < "$PROMPT_FILE"
}

# Serialize on the single local runtime when flock is available; without it, run
# unserialized rather than refusing (the ceiling still bounds the damage).
if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK"
    if ! flock -w "$TIMEOUT_SECS" 9; then
        _refuse 'timed out waiting for the local-model lock (capacity-1)'
    fi
    _run_dispatch
    _rc=$?
    flock -u 9
else
    _run_dispatch
    _rc=$?
fi

if [ "$_rc" -eq 124 ] || [ "$_rc" -eq 143 ]; then
    printf 'local-dispatch: exceeded %ss ceiling\n' "$TIMEOUT_SECS" >&2
    exit 4
fi
exit "$_rc"
