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
# The third precondition bounds the wall clock; this script ALSO bounds silence,
# which is a different failure and needs its own code. A local model that goes
# quiet is not slow, it is overthinking: the observed case failed a `git push`
# then argued with itself until the 600s ceiling killed it, having emitted no
# answer. Slowness must not be punished (a 32B model on a laptop is slow by
# definition, and progress is the only honest signal available), so progress is
# watched by the size of the executor's own output and only its ABSENCE aborts.
#
# Usage:
#   local-dispatch.sh --model <tag> --prompt-file <path>
#                     [--provider ollama|lmstudio] [--cwd <dir>]
#                     [--timeout-secs N] [--stall-secs N] [--skip-capability-check]
#                     [--dry-run]
#                     [--ledger PATH] [--profile NAME] [--kind NAME] [--issue N]
#                     [--labels CSV] [--dispatch-id ID]
#
# Exit codes:
#   0  dispatch completed (stdout is the executor's output)
#   1  bad arguments
#   3  precondition failed — caller MUST fall back to its cloud tier
#   4  dispatch exceeded the wall-clock ceiling
#   5  no-progress abort: STALL_SECS of silence, and THIS script killed it.
#      Reserved — an executor exiting 5 of its own accord passes through as 5
#      only if a stall was not also detected.
#   >4 the executor's own non-zero status
#
# Environment:
#   AUTOSPEC_LOCAL_PROVIDER        default provider (ollama)
#   AUTOSPEC_LOCAL_TIMEOUT_SECS    default ceiling (600)
#   AUTOSPEC_LOCAL_STALL_SECS      no-progress window (120)
#   AUTOSPEC_MODEL_CAPABILITY      probe document path
#   AUTOSPEC_LOCAL_LOCK_DIR        lock directory (default ~/.autospec/locks)
#   AUTOSPEC_ROUTING_LEDGER        ledger a no-progress abort is recorded in
#   AUTOSPEC_RUN_ID                run the abort is attributed to

set -u

MODEL=""
PROMPT_FILE=""
PROVIDER="${AUTOSPEC_LOCAL_PROVIDER:-ollama}"
WORKDIR="."
TIMEOUT_SECS="${AUTOSPEC_LOCAL_TIMEOUT_SECS:-600}"
# The silence window, deliberately generous: a false abort costs a cell its local
# tier for the rest of the run, a missed abort costs at most STALL_SECS.
STALL_SECS="${AUTOSPEC_LOCAL_STALL_SECS:-120}"
SKIP_CAP=0
DRY_RUN=0
CAPABILITY="${AUTOSPEC_MODEL_CAPABILITY:-$HOME/.autospec/model-capability.json}"
LOCK_DIR="${AUTOSPEC_LOCAL_LOCK_DIR:-$HOME/.autospec/locks}"
# A no-progress abort is a ROUTING outcome, not merely a failed dispatch — it is
# what tells route-decide.sh to stop offering this cell a local model — so it is
# recorded in the same ledger the router reads. Unset means "record nothing":
# never invent a ledger inside the caller's cwd.
LEDGER="${AUTOSPEC_ROUTING_LEDGER:-}"
RUN_ID="${AUTOSPEC_RUN_ID:-}"
PROFILE=""
DISPATCH_KIND="implementer"
ISSUE="0"
LABELS=""
DISPATCH_ID=""
# Grace between SIGTERM and SIGKILL once the guard fires.
KILL_GRACE_SECS=5
DISPATCH_OUT=""
STALL_FLAG=""
DISPATCH_BYTES=0
DISPATCH_WALL_MS=0
DISPATCH_RC=0
# The watchdog's two scratch files. Cleaned on exit rather than inside
# _run_dispatch, so the stall report above can still read them. No trap existed
# here before, so nothing else is removed — WORKDIR is the CALLER's directory.
_cleanup() {
    rm -f "$DISPATCH_OUT" "$STALL_FLAG" 2>/dev/null
    return 0
}
trap _cleanup EXIT

_die() { printf 'local-dispatch: %s\n' "$1" >&2; exit "${2:-1}"; }
_warn() { printf 'local-dispatch: WARN: %s\n' "$1" >&2; }
_refuse() { printf 'local-dispatch: %s\n' "$1" >&2; exit 3; }

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --model)        MODEL="${2:-}"; shift 2 ;;
        --prompt-file)  PROMPT_FILE="${2:-}"; shift 2 ;;
        --provider)     PROVIDER="${2:-}"; shift 2 ;;
        --cwd)          WORKDIR="${2:-}"; shift 2 ;;
        --timeout-secs) TIMEOUT_SECS="${2:-}"; shift 2 ;;
        --stall-secs) STALL_SECS="${2:-}"; shift 2 ;;
        --ledger)       LEDGER="${2:-}"; shift 2 ;;
        --profile)      PROFILE="${2:-}"; shift 2 ;;
        --kind)         DISPATCH_KIND="${2:-}"; shift 2 ;;
        --issue)        ISSUE="${2:-}"; shift 2 ;;
        --labels)       LABELS="${2:-}"; shift 2 ;;
        --dispatch-id)  DISPATCH_ID="${2:-}"; shift 2 ;;
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
[ "$TIMEOUT_SECS" -ge 1 ] 2>/dev/null || _die "--timeout-secs must be positive: $TIMEOUT_SECS"
case "$STALL_SECS" in
    ''|*[!0-9]*) _die "--stall-secs must be an integer: $STALL_SECS" ;;
esac
[ "$STALL_SECS" -ge 1 ] 2>/dev/null || _die "--stall-secs must be positive: $STALL_SECS"
# Sampling period for the progress watchdog. The abort can land up to one
# interval late, which is noise against a 120s window; short windows (tests,
# tight loops) sample finely so the guard stays responsive there.
if [ "$STALL_SECS" -le 4 ]; then STALL_POLL_SECS=1
elif [ "$STALL_SECS" -le 30 ]; then STALL_POLL_SECS=2
else STALL_POLL_SECS=5; fi
# The abort row is filed against a routing CELL, so it needs that cell's two
# ordinals, read off the issue's ctx/reasoning labels — the same two ordinals
# route-decide.sh parses, and the same separator tolerance (`ctx:64k` as written
# on an issue, `ctx-64k` as typed on a CLI). An abort whose cell is unknown is
# NOT recorded: a row filed against a guessed cell poisons the evidence for
# dispatches that never happened there, and the router would demote a model that
# did nothing wrong.
CELL_CTX=""
CELL_REASONING=""
if [ -n "$LABELS" ]; then
    CELL_CTX="$(printf '%s\n' "$LABELS" | tr ',' '\n' \
        | sed -n 's/^[[:space:]]*ctx[:-]\(32k\|64k\|120k\)[[:space:]]*$/\1/p' | tail -n1)"
    CELL_REASONING="$(printf '%s\n' "$LABELS" | tr ',' '\n' \
        | sed -n 's/^[[:space:]]*reasoning[:-]\(shallow\|medium\|deep\)[[:space:]]*$/\1/p' | tail -n1)"
fi

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

if [ -z "$PROFILE" ]; then PROFILE="$MODEL"; fi
if [ -z "$DISPATCH_ID" ]; then DISPATCH_ID="local-$$-$(date -u +%s)"; fi
case "$ISSUE" in ''|*[!0-9]*) _die "--issue must be an integer: $ISSUE" ;; esac

if [ "$DRY_RUN" -eq 1 ]; then
    printf 'codex %s\n' "$CODEX_ARGS"
    printf 'timeout_secs=%s stall_secs=%s poll_secs=%s cwd=%s prompt_file=%s\n' \
        "$TIMEOUT_SECS" "$STALL_SECS" "$STALL_POLL_SECS" "$WORKDIR" "$PROMPT_FILE"
    exit 0
fi

if [ ! -d "$LOCK_DIR" ]; then mkdir -p "$LOCK_DIR"; fi
LOCK="$LOCK_DIR/local-model.lock"

# ── no-progress watchdog ──────────────────────────────────────────────────────
# Progress == the executor's output growing. The executor writes to DISPATCH_OUT
# and the watchdog only stats that file, so nothing is withheld from the caller
# beyond one sampling interval and no pipe/pty buffering is introduced. Fires at
# most once, escalating TERM (the executor may clean up) to KILL after a grace.
#
# The watchdog runs in the BACKGROUND so the parent's `wait` returns the instant
# the executor exits — a monitor loop in the foreground would add a polling
# interval of dead time to every dispatch, successful ones included.
_watch_stall() {
    _pid="$1"
    _last=-1
    _deadline=$(( $(date -u +%s) + STALL_SECS ))
    while kill -0 "$_pid" 2>/dev/null; do
        sleep "$STALL_POLL_SECS"
        kill -0 "$_pid" 2>/dev/null || return 0
        _bytes="$(wc -c < "$DISPATCH_OUT" 2>/dev/null | tr -d '[:space:]')"
        case "$_bytes" in ''|*[!0-9]*) _bytes=0 ;; esac
        _now="$(date -u +%s)"
        if [ "$_bytes" -gt "$_last" ]; then
            # Output grew: the model is working, reset the window.
            _last="$_bytes"
            _deadline=$(( _now + STALL_SECS ))
            continue
        fi
        [ "$_now" -lt "$_deadline" ] && continue
        # Silent for the whole window. Claim the abort BEFORE signalling, so the
        # parent can tell our kill from the executor's own exit status. The file
        # must be NON-empty: the parent tests it with -s, and mktemp left it at
        # zero bytes, so truncating it here would claim nothing.
        printf 'stall at %s with %s bytes\n' "$_now" "$_bytes" > "$STALL_FLAG"
        kill -TERM "$_pid" 2>/dev/null
        _grace=0
        while kill -0 "$_pid" 2>/dev/null && [ "$_grace" -lt "$KILL_GRACE_SECS" ]; do
            sleep 1
            _grace=$(( _grace + 1 ))
        done
        if kill -0 "$_pid" 2>/dev/null; then
            # Killing `timeout` alone does not reap the executor, which would
            # orphan a model still holding the one GPU this host cannot share.
            command -v pkill >/dev/null 2>&1 && pkill -KILL -P "$_pid" 2>/dev/null
            kill -KILL "$_pid" 2>/dev/null
        fi
        return 0
    done
    return 0
}

_run_dispatch() {
    DISPATCH_OUT="$(mktemp "${TMPDIR:-/tmp}/autospec-local-out-XXXXXX")"
    STALL_FLAG="$(mktemp "${TMPDIR:-/tmp}/autospec-stall-XXXXXX")"
    # shellcheck disable=SC2086
    timeout --preserve-status "$TIMEOUT_SECS" \
        codex $CODEX_ARGS --cd "$WORKDIR" < "$PROMPT_FILE" > "$DISPATCH_OUT" 2>&1 &
    _pid=$!
    _watch_stall "$_pid" &
    _wd=$!
    _t0="$(date -u +%s)"
    _rc=0
    wait "$_pid" || _rc=$?
    # Reap the watchdog before it can act on a pid that is already gone.
    kill "$_wd" 2>/dev/null
    wait "$_wd" 2>/dev/null
    # Drain whatever the executor wrote after the last sample — the caller sees
    # the full output whether the dispatch finished or was killed.
    cat "$DISPATCH_OUT" 2>/dev/null
    _b="$(wc -c < "$DISPATCH_OUT" 2>/dev/null | tr -d '[:space:]')"
    case "$_b" in ''|*[!0-9]*) _b=0 ;; esac
    DISPATCH_BYTES="$_b"
    DISPATCH_WALL_MS=$(( ($(date -u +%s) - _t0) * 1000 ))
    DISPATCH_RC="$_rc"
    # The caller reads this return value; an assignment as the last statement
    # would hand back 0 and report every killed dispatch as a clean one.
    return "$_rc"
}

# Record the no-progress abort in the routing ledger. This row is the only thing
# that turns a killed dispatch into a routing decision, so a failure to write it
# is loud — but never fatal: the exit code the caller acts on is already decided.
_record_abort() {
    if [ -z "$LEDGER" ]; then
        _warn 'no ledger configured (AUTOSPEC_ROUTING_LEDGER); abort not recorded'
        return 0
    fi
    if [ -z "$CELL_CTX" ] || [ -z "$CELL_REASONING" ]; then
        _warn 'cell ordinals unknown (no ctx-*/reasoning-* labels); abort not recorded'
        return 0
    fi
    _sh="$(dirname "$0")/routing-ledger.sh"
    if [ ! -f "$_sh" ] || ! command -v jq >/dev/null 2>&1; then
        _warn 'routing-ledger.sh or jq unavailable; abort not recorded'
        return 0
    fi
    _row="$(jq -nc \
        --arg id "$DISPATCH_ID" --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --arg kind "$DISPATCH_KIND" --arg profile "$PROFILE" --arg model "$MODEL" \
        --arg ctx "$CELL_CTX" --arg reasoning "$CELL_REASONING" --arg run "$RUN_ID" \
        --argjson issue "$ISSUE" --argjson ms "$DISPATCH_WALL_MS" \
        '{dispatch_id:$id,ts:$ts,dispatch_kind:$kind,profile:$profile,model:$model,
          harness:"codex-oss",issue:$issue,cell_ctx:$ctx,cell_reasoning:$reasoning,
          input_tokens:0,output_tokens:0,cached_tokens:0,wall_clock_ms:$ms,
          retries:0,escalated:true,outcome:"local_overthink_abort",
          reason:"no output for the stall window",
          run_id:$run}')" || { _warn 'could not build the abort row'; return 0; }
    if bash "$_sh" --ledger "$LEDGER" --append "$_row" >/dev/null 2>&1; then
        _warn "recorded local_overthink_abort in $LEDGER (cell $CELL_CTX/$CELL_REASONING, profile $PROFILE)"
    else
        _warn 'ledger append failed for local_overthink_abort'
    fi
    return 0
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

# The stall guard and the wall clock must never be confused: exit 4 means "ran
# out of time", exit 5 means "stopped making progress", and the router demotes on
# the latter only. A stall is reported only when the watchdog claims it AND the
# executor died from the signal we sent — so an executor that finishes cleanly in
# the instant the guard fires is still reported as a success.
_STALLED=0
if [ -s "$STALL_FLAG" ]; then
    if [ "$_rc" -eq 143 ] || [ "$_rc" -eq 137 ]; then
        _STALLED=1
    else
        _warn "stall flag set but the executor exited $_rc; reporting its status"
    fi
fi

if [ "$_STALLED" -eq 1 ]; then
    printf 'local-dispatch: local_overthink_abort — no output for %ss (killed after %ss of %ss ceiling, %s bytes emitted)\n' \
        "$STALL_SECS" "$(( DISPATCH_WALL_MS / 1000 ))" "$TIMEOUT_SECS" "$DISPATCH_BYTES" >&2
    _record_abort
    exit 5
fi

if [ "$_rc" -eq 124 ] || [ "$_rc" -eq 143 ]; then
    printf 'local-dispatch: exceeded %ss ceiling\n' "$TIMEOUT_SECS" >&2
    exit 4
fi
exit "$_rc"
