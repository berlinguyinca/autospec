#!/usr/bin/env bash
# scripts/executor-dispatch.sh — the §16 provider-neutral executor abstraction.
#
# One contract, `dispatch(request) → result`, across every harness, so that
# orchestration never special-cases a provider. Spec §16 names the request and
# result fields; this script is their only implementation.
#
# Why a wrapper rather than a rewrite: AS-AEO-001 §72.2 requires existing
# provider and execution paths to be wrapped behind the new interfaces *before*
# behaviour is replaced. `dispatch-implementer.sh` keeps owning worktree
# isolation for Phase 4; this script owns the request/result envelope, and a
# cloud-only single-provider dispatch routes exactly as it does today (§52).
#
# The one rule that outranks convenience: a metric the adapter did not observe
# is the string "unknown", never 0 and never estimated (§25). A fabricated zero
# is indistinguishable from a measured zero once it reaches the routing ledger,
# and it makes a provider look infinitely slow. `wall_clock_ms` is the only
# metric this script measures itself, so it is the only one always numeric.
#
# Usage:
#   executor-dispatch.sh --request <file.json> [--dry-run]
#   executor-dispatch.sh --schema-path
#
# The request file carries the §16 fields:
#   work_item role dispatch_kind model provider context_budget tools
#   workspace acceptance_criteria timeout
#
# stdout is the §16 result envelope, validating against
# schemas/autospec-dispatch-result.schema.json:
#   schema status output patch input_tokens output_tokens cached_tokens
#   prompt_tok_s decode_tok_s ttft_ms wall_clock_ms tool_calls failure_class
#
# Providers (harnesses, one spelling each): claude codex opencode
# Local runtimes (ollama, lmstudio, vllm, llamacpp) are deliberately NOT
# adapters here; wiring them through this contract is issue #3173's scope, and
# until then they must refuse loudly rather than silently reach a cloud harness.
#
# Exit codes:
#   0   dispatch completed
#   1   usage error / invalid request      (failure_class=invalid_request)
#   2   jq missing — fail closed
#   3   precondition failed                (failure_class=harness_unavailable)
#   4   dispatch exceeded its timeout      (failure_class=timeout)
#   5   the harness itself failed          (failure_class=harness_error)
#   12  unknown provider                   (failure_class=unsupported_provider)
#
# Environment:
#   AUTOSPEC_EXECUTOR_TIMEOUT_SECS    default ceiling when the request omits one (600)
#   AUTOSPEC_EXECUTOR_CLAUDE_BIN      override the resolved claude executable
#   AUTOSPEC_EXECUTOR_CODEX_BIN       override the resolved codex executable
#   AUTOSPEC_EXECUTOR_OPENCODE_BIN    override the resolved opencode executable
#   AUTOSPEC_SCHEMAS_DIR              installed schema directory (~/.autospec/schemas)

set -u

SCHEMA_ID="autospec.dispatch-result.v1"
SCHEMA_FILE="autospec-dispatch-result.schema.json"

# The 14 snake_case roles. A role outside this list is a request bug, not a
# routing decision, so it fails closed before any harness is reached.
VALID_ROLES="orchestrator planner architect test_planner implementer"
VALID_ROLES="$VALID_ROLES code_reviewer test_reviewer qa_verifier"
VALID_ROLES="$VALID_ROLES documentation_writer documentation_reviewer"
VALID_ROLES="$VALID_ROLES ui_ux_reviewer security_reviewer researcher advisor"

usage() {
    cat <<'EOF'
Usage: executor-dispatch.sh --request <file.json> [--dry-run]
       executor-dispatch.sh --schema-path

Dispatches one request through the provider-neutral executor contract and
prints the result envelope on stdout.

Providers (harnesses): claude, codex, opencode

Request fields: work_item role dispatch_kind model provider context_budget
                tools workspace acceptance_criteria timeout
Result fields:  status output patch input_tokens output_tokens cached_tokens
                prompt_tok_s decode_tok_s ttft_ms wall_clock_ms tool_calls
                failure_class

Metrics the harness did not report are emitted as "unknown", never 0.

Exit: 0 ok | 1 usage/invalid request | 2 jq missing | 3 harness unavailable
      4 timeout | 5 harness error | 12 unsupported provider
EOF
}

_die() { printf 'executor-dispatch: %s\n' "$1" >&2; exit "${2:-1}"; }

# Resolve the result schema across both layouts it lives in: the repo checkout
# (schemas/ beside scripts/) and the installed tree ($AUTOSPEC_SCHEMAS_DIR).
# Never a bare relative path — that breaks on an installed tree.
SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd -P)" || SCRIPT_DIR=""
_schema_path() {
    for _cand in \
        "$SCRIPT_DIR/../schemas/$SCHEMA_FILE" \
        "${AUTOSPEC_SCHEMAS_DIR:-$HOME/.autospec/schemas}/$SCHEMA_FILE"
    do
        if [ -f "$_cand" ]; then printf '%s' "$_cand"; return 0; fi
    done
    return 1
}

REQUEST_FILE=""
DRY_RUN=0

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)     usage; exit 0 ;;
        --schema-path)
            _found="$(_schema_path)" || _die "no $SCHEMA_FILE found"
            printf '%s\n' "$_found"; exit 0 ;;
        --request)     REQUEST_FILE="${2:-}"; shift 2 ;;
        --dry-run)     DRY_RUN=1; shift ;;
        *)             printf 'executor-dispatch: unknown option: %s\n' "$1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -z "$REQUEST_FILE" ]; then
    printf 'executor-dispatch: --request is required\n' >&2
    usage >&2
    exit 1
fi

# jq is the envelope's only serializer: without it no result can be emitted at
# all, so refuse rather than hand a caller hand-rolled JSON of unknown validity.
if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required to build the result envelope' 2
fi

# ── clock ─────────────────────────────────────────────────────────────────────
# GNU `date +%s%3N` is unavailable on macOS, so prefer perl's Time::HiRes and
# degrade to whole seconds rather than reporting a fabricated sub-second value.
_now_ms() {
    _ms="$(perl -MTime::HiRes -e 'printf("%.0f", Time::HiRes::time()*1000)' 2>/dev/null)"
    case "${_ms:-}" in
        ''|*[!0-9]*) _ms="$(( $(date +%s) * 1000 ))" ;;
    esac
    printf '%s' "$_ms"
}

START_MS="$(_now_ms)"

# ── result envelope ───────────────────────────────────────────────────────────
# Every metric starts at "unknown" and is only ever overwritten by a value the
# harness actually reported. This is the direction that makes fabrication a
# deliberate act rather than an oversight.
R_OUTPUT=""
R_PATCH="unknown"
R_INPUT_TOKENS="unknown"
R_OUTPUT_TOKENS="unknown"
R_CACHED_TOKENS="unknown"
R_PROMPT_TOK_S="unknown"
R_DECODE_TOK_S="unknown"
R_TTFT_MS="unknown"
R_TOOL_CALLS="unknown"

_emit_and_exit() {
    # $1 status, $2 failure_class, $3 exit code
    _wall="$(( $(_now_ms) - START_MS ))"
    if [ "$_wall" -lt 0 ]; then _wall=0; fi
    jq -n \
        --arg schema        "$SCHEMA_ID" \
        --arg status        "$1" \
        --arg failure_class "$2" \
        --arg output        "$R_OUTPUT" \
        --arg patch         "$R_PATCH" \
        --arg input_tokens  "$R_INPUT_TOKENS" \
        --arg output_tokens "$R_OUTPUT_TOKENS" \
        --arg cached_tokens "$R_CACHED_TOKENS" \
        --arg prompt_tok_s  "$R_PROMPT_TOK_S" \
        --arg decode_tok_s  "$R_DECODE_TOK_S" \
        --arg ttft_ms       "$R_TTFT_MS" \
        --arg tool_calls    "$R_TOOL_CALLS" \
        --argjson wall_clock_ms "$_wall" \
        'def metric: if . == "unknown" then . else tonumber end;
         {
           schema:         $schema,
           status:         $status,
           output:         $output,
           patch:          $patch,
           input_tokens:   ($input_tokens  | metric),
           output_tokens:  ($output_tokens | metric),
           cached_tokens:  ($cached_tokens | metric),
           prompt_tok_s:   ($prompt_tok_s  | metric),
           decode_tok_s:   ($decode_tok_s  | metric),
           ttft_ms:        ($ttft_ms       | metric),
           wall_clock_ms:  $wall_clock_ms,
           tool_calls:     ($tool_calls    | metric),
           failure_class:  $failure_class
         }'
    exit "$3"
}

_invalid() { R_OUTPUT="$1"; _emit_and_exit failure invalid_request 1; }

# ── request ───────────────────────────────────────────────────────────────────
if [ ! -r "$REQUEST_FILE" ]; then
    _invalid "request file is not readable: $REQUEST_FILE"
fi
if ! jq -e 'type == "object"' "$REQUEST_FILE" >/dev/null 2>&1; then
    _invalid "request is not a JSON object: $REQUEST_FILE"
fi

_field() {
    jq -r --arg k "$1" \
        'if (has($k) and .[$k] != null) then (.[$k] | tostring) else "" end' \
        "$REQUEST_FILE" 2>/dev/null
}

WORK_ITEM="$(_field work_item)"
ROLE="$(_field role)"
DISPATCH_KIND="$(_field dispatch_kind)"
MODEL="$(_field model)"
PROVIDER="$(_field provider)"
CONTEXT_BUDGET="$(_field context_budget)"
WORKSPACE="$(_field workspace)"
ACCEPTANCE="$(_field acceptance_criteria)"
TIMEOUT_SECS="$(_field timeout)"
TOOLS_CSV="$(jq -r '(.tools // []) | map(tostring) | join(",")' "$REQUEST_FILE" 2>/dev/null)"

for _pair in "work_item:$WORK_ITEM" "role:$ROLE" "dispatch_kind:$DISPATCH_KIND" \
             "provider:$PROVIDER" "workspace:$WORKSPACE"; do
    if [ -z "${_pair#*:}" ]; then
        _invalid "request is missing required field: ${_pair%%:*}"
    fi
done

case " $VALID_ROLES " in
    *" $ROLE "*) ;;
    *) _invalid "role is outside the 14-role vocabulary: $ROLE" ;;
esac

if [ ! -d "$WORKSPACE" ]; then
    _invalid "workspace is not a directory: $WORKSPACE"
fi

if [ -z "$TIMEOUT_SECS" ]; then
    TIMEOUT_SECS="${AUTOSPEC_EXECUTOR_TIMEOUT_SECS:-600}"
fi
case "$TIMEOUT_SECS" in
    ''|*[!0-9]*) _invalid "timeout must be a positive integer of seconds: $TIMEOUT_SECS" ;;
esac
if [ "$TIMEOUT_SECS" -le 0 ]; then
    _invalid "timeout must be a positive integer of seconds: $TIMEOUT_SECS"
fi

# ── adapter selection ─────────────────────────────────────────────────────────
# #3173 adds local-runtime adapters here. Until then an unrecognised provider —
# including a valid runtime name — refuses with exit 12 rather than falling
# through to a harness the caller did not ask for.
case "$PROVIDER" in
    claude)   BIN_ENV="AUTOSPEC_EXECUTOR_CLAUDE_BIN" ;;
    codex)    BIN_ENV="AUTOSPEC_EXECUTOR_CODEX_BIN" ;;
    opencode) BIN_ENV="AUTOSPEC_EXECUTOR_OPENCODE_BIN" ;;
    *)
        R_OUTPUT="no adapter for provider: $PROVIDER (claude|codex|opencode)"
        _emit_and_exit failure unsupported_provider 12
        ;;
esac

BIN="${!BIN_ENV:-}"
if [ -z "$BIN" ]; then
    BIN="$(command -v "$PROVIDER" 2>/dev/null || true)"
fi
if [ -z "$BIN" ]; then
    R_OUTPUT="harness binary not found: $PROVIDER (set $BIN_ENV or put it on PATH)"
    _emit_and_exit failure harness_unavailable 3
fi

RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/executor-dispatch-XXXXXX")" || \
    _die 'could not create a working directory' 3
trap 'rm -rf "$RUN_DIR"' EXIT
STDOUT_FILE="$RUN_DIR/stdout"
ARTIFACT="$RUN_DIR/last-message.txt"
TIMED_OUT="$RUN_DIR/timed-out"

PROMPT="$(printf 'Work item: %s\nRole: %s\nDispatch kind: %s\nContext budget: %s\nWorkspace: %s\n\nAcceptance criteria:\n%s\n' \
    "$WORK_ITEM" "$ROLE" "$DISPATCH_KIND" "${CONTEXT_BUDGET:-unknown}" "$WORKSPACE" "${ACCEPTANCE:-none stated}")"

ARGV=()
case "$PROVIDER" in
    claude)
        # `--output-format json` is what makes real token counts observable at
        # all; without it every count would be "unknown" forever.
        ARGV=("$BIN" -p --output-format json --permission-mode acceptEdits
              --no-session-persistence)
        if [ -n "$MODEL" ]; then ARGV+=(--model "$MODEL"); fi
        if [ -n "$TOOLS_CSV" ]; then ARGV+=(--allowedTools "$TOOLS_CSV"); fi
        ARGV+=("$PROMPT")
        ;;
    codex)
        # --skip-git-repo-check: a headless dispatch may target a workspace that
        # is not a git repository, and codex exec refuses one without this.
        ARGV=("$BIN" exec --skip-git-repo-check -C "$WORKSPACE"
              --sandbox workspace-write --ephemeral
              --output-last-message "$ARTIFACT")
        if [ -n "$MODEL" ]; then ARGV+=(--model "$MODEL"); fi
        ARGV+=("$PROMPT")
        ;;
    opencode)
        ARGV=("$BIN" --pure run)
        if [ -n "$MODEL" ]; then ARGV+=(--model "$MODEL"); fi
        ARGV+=("$PROMPT")
        ;;
esac

if [ "$DRY_RUN" -eq 1 ]; then
    printf '%s\n' "${ARGV[@]}"
    printf 'timeout_secs=%s workspace=%s provider=%s\n' \
        "$TIMEOUT_SECS" "$WORKSPACE" "$PROVIDER"
    exit 0
fi

# ── bounded execution ─────────────────────────────────────────────────────────
# The ceiling is enforced by a portable watchdog rather than timeout(1), which
# is absent on a stock macOS. The watchdog records a sentinel file before it
# kills, so "we timed out" is never inferred from an exit status the harness
# could also produce on its own.
( cd "$WORKSPACE" && exec "${ARGV[@]}" ) >"$STDOUT_FILE" 2>/dev/null </dev/null &
CHILD_PID=$!
(
    _elapsed=0
    while [ "$_elapsed" -lt "$TIMEOUT_SECS" ]; do
        sleep 1
        if ! kill -0 "$CHILD_PID" 2>/dev/null; then exit 0; fi
        _elapsed=$(( _elapsed + 1 ))
    done
    : > "$TIMED_OUT"
    kill -TERM "$CHILD_PID" 2>/dev/null
    sleep 2
    kill -KILL "$CHILD_PID" 2>/dev/null
) >/dev/null 2>&1 &
WATCHDOG_PID=$!

# The job-termination notice bash prints when the watchdog kills the child would
# otherwise interleave with the envelope on a caller that merges the two streams.
wait "$CHILD_PID" 2>/dev/null
HARNESS_RC=$?
kill -TERM "$WATCHDOG_PID" 2>/dev/null
wait "$WATCHDOG_PID" 2>/dev/null

RAW_STDOUT="$(cat "$STDOUT_FILE" 2>/dev/null)"

# ── result extraction ─────────────────────────────────────────────────────────
# Read one `usage` counter out of a Claude JSON result. Anything absent, null or
# non-numeric is "unknown"; there is no path here that yields a default 0.
_claude_usage() {
    _u="$(printf '%s' "$RAW_STDOUT" | jq -r --arg k "$1" \
        'if (.usage? // null) != null and (.usage[$k] // null) != null
         then (.usage[$k] | tostring) else "unknown" end' 2>/dev/null)"
    case "${_u:-}" in
        ''|*[!0-9]*) printf 'unknown' ;;
        *)           printf '%s' "$_u" ;;
    esac
}

case "$PROVIDER" in
    claude)
        # Parse only what the harness actually reported. An unparseable or
        # truncated stream degrades to the raw text with every count left
        # "unknown" — never to zeroes.
        if printf '%s' "$RAW_STDOUT" | jq -e 'type == "object"' >/dev/null 2>&1; then
            R_OUTPUT="$(printf '%s' "$RAW_STDOUT" | jq -r '.result // ""' 2>/dev/null)"
            R_INPUT_TOKENS="$(_claude_usage input_tokens)"
            R_OUTPUT_TOKENS="$(_claude_usage output_tokens)"
            R_CACHED_TOKENS="$(_claude_usage cache_read_input_tokens)"
        else
            R_OUTPUT="$RAW_STDOUT"
        fi
        ;;
    codex)
        if [ -s "$ARTIFACT" ]; then
            R_OUTPUT="$(cat "$ARTIFACT" 2>/dev/null)"
        else
            R_OUTPUT="$RAW_STDOUT"
        fi
        ;;
    opencode)
        R_OUTPUT="$RAW_STDOUT"
        ;;
esac

# `patch` stays "unknown" when the workspace is not a git worktree: an empty
# string there would claim "the harness changed nothing", which is not observed.
if git -C "$WORKSPACE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    R_PATCH="$(git -C "$WORKSPACE" diff HEAD 2>/dev/null)"
    if [ -z "$R_PATCH" ]; then
        R_PATCH="$(git -C "$WORKSPACE" diff 2>/dev/null)"
    fi
fi

if [ -f "$TIMED_OUT" ]; then
    _emit_and_exit timeout timeout 4
fi
if [ "$HARNESS_RC" -ne 0 ]; then
    _emit_and_exit failure harness_error 5
fi
_emit_and_exit success none 0
