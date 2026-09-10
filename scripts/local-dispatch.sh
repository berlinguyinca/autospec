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
# swap and both blow their ceiling. A host-scoped lock serializes them. The
# lock's lifetime is exactly the dispatch process's (the executor never inherits
# the lock descriptor), and the lock records its holder (pid + start time) so a
# refusal names who holds it and since when instead of dead-ending.
#
# Post-dispatch check (R8, tracker #3344) — verbatim-anchor verification:
# a local model can silently substitute one word in the source text it
# paraphrases ("persistent agent" written as "persistence agent") with no
# tell. Every claim a local dispatch makes about source text must carry a
# `file:line` anchor AND a verbatim quote span, and `--verify-anchors` checks
# the quote against the named source: a claim is kept only when the span
# exists verbatim at the named location (that exact line when one is given,
# anywhere in the file otherwise). Unanchored, unresolvable, and non-matching
# claims — including one-word near-misses — are DROPPED, never escalated: a
# human triaging fabricated quotes is the cost this rule avoids. Exit stays
# 0; stdout is the kept-claims JSON array and stderr carries one line per
# drop plus the summary `local-dispatch: anchor-verify kept=N dropped=M
# total=T` so the caller can record the drop count on the routing ledger
# (`routing-ledger.sh` optional `anchor_drops` key).
#
# Usage:
#   local-dispatch.sh --model <tag> --prompt-file <path>
#                     [--provider ollama|lmstudio] [--cwd <dir>]
#                     [--timeout-secs N] [--skip-capability-check] [--dry-run]
#   local-dispatch.sh --verify-anchors <claims.json|-> [--root <dir>]
#
# Exit codes:
#   0  dispatch completed (stdout is the executor's output)
#   1  bad arguments
#   2  --verify-anchors: jq missing (fail-closed)
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
VERIFY_CLAIMS=""
ROOT_DIR="."

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
        --verify-anchors) VERIFY_CLAIMS="${2:-}"; shift 2 ;;
        --root)         ROOT_DIR="${2:-}"; shift 2 ;;
        *) _die "unknown option: $1" ;;
    esac
done

# --model/--prompt-file are dispatch-mode requirements only; --verify-anchors
# is a standalone post-dispatch check and needs neither.
if [ -z "$VERIFY_CLAIMS" ]; then
    if [ -z "$MODEL" ]; then _die '--model is required'; fi
    if [ -z "$PROMPT_FILE" ]; then _die '--prompt-file is required'; fi
    if [ ! -f "$PROMPT_FILE" ]; then _die "prompt file not found: $PROMPT_FILE"; fi
fi
case "$PROVIDER" in
    ollama|lmstudio) ;;
    *) _die "unsupported provider: $PROVIDER (ollama|lmstudio)" ;;
esac
case "$TIMEOUT_SECS" in
    ''|*[!0-9]*) _die "--timeout-secs must be an integer: $TIMEOUT_SECS" ;;
esac

# ── R8: verbatim-anchor verification of local paraphrase output ─────────────

_trim() {
    _t="$1"
    _t="${_t#"${_t%%[![:space:]]*}"}"
    _t="${_t%"${_t##*[![:space:]]}"}"
    printf '%s' "$_t"
}

# _verify_one_claim <claim-json> — print a drop reason; empty means keep.
# A claim is kept only when BOTH `quote` and `anchor` are non-empty AND the
# quote exists verbatim at the named location. The match is exact: case- and
# whitespace-sensitive, line-bound when a line is given — a one-word
# near-miss never matches.
_verify_one_claim() {
    if ! printf '%s' "$1" | jq -e 'type == "object"' >/dev/null 2>&1; then
        printf 'claim is not a JSON object'
        return 0
    fi
    _q="$(_trim "$(printf '%s' "$1" | jq -r '(.quote // "") | tostring')")"
    _a="$(_trim "$(printf '%s' "$1" | jq -r '(.anchor // "") | tostring')")"
    if [ -z "$_q" ] || [ -z "$_a" ]; then
        printf 'unanchored claim (needs both a file:line anchor and a verbatim quote): %s' \
            "$(printf '%s' "$1" | jq -r '(.claim // "") | tostring | .[0:120]')"
        return 0
    fi
    case "$_a" in
        *:*) _path="${_a%:*}"; _line="${_a##*:}" ;;
        *)   _path="$_a"; _line="" ;;
    esac
    case "$_path" in
        ''|/*|.|..)
            printf 'unresolvable anchor path: %s' "$_a"
            return 0 ;;
    esac
    case "$_path" in
        ../*|*/../*|*/..)
            printf 'unresolvable anchor path (traversal): %s' "$_a"
            return 0 ;;
    esac
    if [ -n "$_line" ]; then
        case "$_line" in
            *[!0-9]*)
                printf 'anchor line is not a positive integer: %s' "$_a"
                return 0 ;;
        esac
    fi
    _src="$ROOT_DIR/$_path"
    if [ ! -f "$_src" ]; then
        printf 'source not found at %s' "$_path"
        return 0
    fi
    if [ -n "$_line" ]; then
        if ! printf '%s\n' "$(sed -n "${_line}p" "$_src" 2>/dev/null)" | grep -F -q -e "$_q"; then
            printf 'quote not verbatim at %s:%s (near-miss or wrong line)' "$_path" "$_line"
            return 0
        fi
    else
        if ! grep -F -q -e "$_q" "$_src" 2>/dev/null; then
            printf 'quote not verbatim in %s' "$_path"
            return 0
        fi
    fi
    return 0
}

# _run_verify_anchors — the --verify-anchors mode. Dropped claims never fail
# the run: exit 0 with the kept array on stdout, so the caller records the
# drop count on the ledger instead of escalating fabricated quotes to a human.
_run_verify_anchors() {
    if ! command -v jq >/dev/null 2>&1; then
        printf 'local-dispatch: jq is required for --verify-anchors (fail-closed)\n' >&2
        exit 2
    fi
    if [ "$VERIFY_CLAIMS" = "-" ]; then
        _claims="$(cat)"
    else
        _claims="$(cat "$VERIFY_CLAIMS")"
    fi
    if ! printf '%s' "$_claims" | jq -e 'type == "array"' >/dev/null 2>&1; then
        _die "claims input must be a JSON array of {claim, anchor, quote} objects"
    fi
    _total="$(printf '%s' "$_claims" | jq 'length')"
    _kept='[]'
    _dropped=0
    _i=0
    while [ "$_i" -lt "$_total" ]; do
        _claim="$(printf '%s' "$_claims" | jq -c ".[$_i]")"
        _reason="$(_verify_one_claim "$_claim")"
        if [ -z "$_reason" ]; then
            _kept="$(printf '%s' "$_kept" | jq -c --argjson c "$_claim" '. + [$c]')"
        else
            _dropped=$((_dropped + 1))
            printf 'local-dispatch: anchor-verify drop: %s\n' "$_reason" >&2
        fi
        _i=$((_i + 1))
    done
    printf 'local-dispatch: anchor-verify kept=%s dropped=%s total=%s\n' \
        "$(printf '%s' "$_kept" | jq 'length')" "$_dropped" "$_total" >&2
    printf '%s' "$_kept"
}

if [ -n "$VERIFY_CLAIMS" ]; then
    if [ ! -d "$ROOT_DIR" ]; then _die "root directory not found: $ROOT_DIR"; fi
    if [ "$VERIFY_CLAIMS" != "-" ] && [ ! -f "$VERIFY_CLAIMS" ]; then
        _die "claims file not found: $VERIFY_CLAIMS"
    fi
    _run_verify_anchors
    exit $?
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
    # A stale or hand-edited document can claim dispatch_recommended=true for a
    # model while the accelerator block records an ambiguous state (present but
    # not provably usable). The probe never produces that shape — ambiguous
    # forces every entry to false — but the document is read from disk, so the
    # claim is cross-checked against the accelerator fact it is derived from.
    _ok="$(jq -r --arg m "$MODEL" '
        (.accelerator.usable // false) as $usable
        | (.local_models // []) | map(select(.model == $m)) | first
        | if . == null then "absent"
          elif .dispatch_recommended != true then "no"
          elif $usable != true then "ambiguous"
          else "yes" end' \
        "$CAPABILITY" 2>/dev/null || printf 'absent')"
    case "$_ok" in
        yes) ;;
        no)  _refuse "model $MODEL is present but not dispatch_recommended ($(jq -r '.accelerator.reason // "unknown"' "$CAPABILITY" 2>/dev/null))" ;;
        ambiguous)
          _refuse "model $MODEL is dispatch_recommended but the accelerator is not provably usable ($(jq -r '.accelerator.reason // "unknown"' "$CAPABILITY" 2>/dev/null))" ;;
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
    # 9>&-: the lock lives on the open file description, and every child
    # inherits fd 9 across fork. A long-lived descendant of the executor
    # (a preview server, an agent subprocess) would otherwise keep the lock
    # held long after this dispatch — and after this process — is gone.
    # Closing it here makes the lock's lifetime exactly this process's.
    # shellcheck disable=SC2086
    timeout --preserve-status "$TIMEOUT_SECS" \
        codex $CODEX_ARGS --cd "$WORKDIR" < "$PROMPT_FILE" 9>&-
}

# Side-effect free: it reads the holder record and /proc only and never
# acquires the lock, so asking "who holds it" cannot change the answer.
describe_lock_holder() {
    local line pid since cmdline note
    line="$(sed -n '$p' "$LOCK" 2>/dev/null || true)"
    pid="${line#pid=}"; pid="${pid%% *}"
    [[ "$pid" =~ ^[0-9]+$ ]] || return 0
    since="${line##*since=}"; since="${since%% *}"
    note="held by recorded pid $pid"
    if [ -r "/proc/$pid/cmdline" ]; then
        # Liveness AND identity: a recycled pid must not be mistaken for
        # the holder just because it happens to be alive.
        cmdline="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | sed 's/ *$//')"
        if [[ "$cmdline" == *local-dispatch* ]]; then
            note="$note (running local-dispatch)"
        else
            note="$note, but that pid is not running local-dispatch (cmdline: ${cmdline:-unknown}); the record may name a recycled pid"
        fi
    elif kill -0 "$pid" 2>/dev/null; then
        note="$note (alive; identity unverifiable without /proc)"
    else
        note="$note is gone; the lock leaked through a descriptor its descendant inherited — find the holder with: lsof $LOCK"
    fi
    [[ "$since" =~ ^[0-9]+$ ]] && note="$note since $since"
    printf '%s' "$note"
    return 0
}

# Serialize on the single local runtime when flock is available; without it, run
# unserialized rather than refusing (the ceiling still bounds the damage).
#
# The lock's lifetime must be exactly this process's: the file is appended
# (a truncate would erase the LIVE holder's record before we know whether we
# can take the lock), the holder is recorded with its pid and start time only
# after the lock is held, and the record is cleared on release so a dead
# holder cannot outlive the dispatch.
if command -v flock >/dev/null 2>&1; then
    exec 9>>"$LOCK"
    if ! flock -w "$TIMEOUT_SECS" 9; then
        holder="$(describe_lock_holder)"
        if [ -n "$holder" ]; then
            _refuse "timed out waiting for the local-model lock (capacity-1): $holder"
        fi
        _refuse 'timed out waiting for the local-model lock (capacity-1)'
    fi
    printf 'pid=%s since=%s prog=local-dispatch\n' "$$" "$(date +%s)" >&9
    _run_dispatch
    _rc=$?
    flock -u 9
    : > "$LOCK"
else
    _run_dispatch
    _rc=$?
fi

if [ "$_rc" -eq 124 ] || [ "$_rc" -eq 143 ]; then
    printf 'local-dispatch: exceeded %ss ceiling\n' "$TIMEOUT_SECS" >&2
    exit 4
fi
exit "$_rc"
