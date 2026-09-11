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
# Runtime adapter mode (issue #3173, spec §16/§17): `--runtime <ollama|
# lmstudio|vllm|llamacpp>` dispatches through a direct OpenAI-compatible
# adapter instead of Codex — the runtimes Codex's --local-provider does not
# cover (vllm, llamacpp) speak the same /v1/chat/completions shape. The
# endpoint comes from the model-supply probe (probe_runtimes), never from a
# name or a hardcoded address; an unreachable endpoint exits 3 so the caller
# keeps its cloud tier. The three preconditions above and the capacity-1 host
# lock apply to this mode unchanged. stdout is the §16 result envelope — the
# same keys as scripts/executor-dispatch.sh, validated by
# schemas/autospec-dispatch-result.schema.json — and any metric the response
# did not report is "unknown", never 0. Token counts are read from the
# response's usage block; prompt_tok_s and decode_tok_s are measured from
# those counters and the request timing, never a constant.
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
#   local-dispatch.sh --runtime <ollama|lmstudio|vllm|llamacpp> --model <tag>
#                     --prompt-file <path> [--timeout-secs N]
#                     [--skip-capability-check] [--dry-run]
#   local-dispatch.sh --verify-anchors <claims.json|-> [--root <dir>]
#
# Exit codes:
#   0  dispatch completed (codex path: stdout is the executor's output;
#      runtime path: stdout is the §16 result envelope)
#   1  bad arguments
#   2  jq missing (fail-closed; --verify-anchors and the runtime path need it)
#   3  precondition failed — caller MUST fall back to its cloud tier
#   4  dispatch exceeded the wall-clock ceiling
#   5  runtime path: the endpoint or the model failed (failure_class=harness_error)
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
RUNTIME=""
PROVIDER_SET=0

_die() { printf 'local-dispatch: %s\n' "$1" >&2; exit "${2:-1}"; }
_refuse() {
    printf 'local-dispatch: %s\n' "$1" >&2
    # Runtime mode speaks the §16 contract: a refusal is still a result, so the
    # envelope carries the diagnostic instead of leaving the caller to guess.
    if [ -n "$RUNTIME" ]; then
        R_OUTPUT="$1"
        _finish failure precondition_failed 3
    fi
    exit 3
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --model)        MODEL="${2:-}"; shift 2 ;;
        --prompt-file)  PROMPT_FILE="${2:-}"; shift 2 ;;
        --provider)     PROVIDER="${2:-}"; PROVIDER_SET=1; shift 2 ;;
        --runtime)      RUNTIME="${2:-}"; shift 2 ;;
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
# The four runtime spellings are exactly what probe_runtimes() emits —
# llamacpp, never llama.cpp. A name the probe cannot know about is a usage
# error, not a guess: a misspelling that reached a hardcoded address would
# dispatch to the wrong endpoint.
if [ -n "$RUNTIME" ] && [ "$PROVIDER_SET" -eq 1 ]; then
    _die '--runtime and --provider are mutually exclusive'
fi
case "$RUNTIME" in
    '') ;;
    ollama|lmstudio|vllm|llamacpp) ;;
    *) _die "unsupported runtime: $RUNTIME (ollama|lmstudio|vllm|llamacpp)" ;;
esac
if [ -z "$RUNTIME" ]; then
    case "$PROVIDER" in
        ollama|lmstudio) ;;
        *) _die "unsupported provider: $PROVIDER (ollama|lmstudio)" ;;
    esac
fi
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

# ── runtime adapter mode (#3173): the §16 result envelope ─────────────────────
# Every metric starts at "unknown" and is only ever overwritten by a value the
# response actually reported — a fabricated zero is indistinguishable from a
# measured zero once it reaches the routing ledger.
if [ -n "$RUNTIME" ]; then
    # The envelope has no serializer but jq; refuse before any other check.
    if ! command -v jq >/dev/null 2>&1; then
        _die 'jq is required for the runtime adapter result envelope' 2
    fi
    R_OUTPUT=""
    R_PATCH="unknown"
    R_INPUT_TOKENS="unknown"
    R_OUTPUT_TOKENS="unknown"
    R_CACHED_TOKENS="unknown"
    R_PROMPT_TOK_S="unknown"
    R_DECODE_TOK_S="unknown"
    R_TTFT_MS="unknown"
    R_TOOL_CALLS="unknown"
    _HTTP_CODE=""
    START_MS="$(perl -MTime::HiRes -e 'printf("%.0f", Time::HiRes::time()*1000)' 2>/dev/null)"
    case "${START_MS:-}" in
        ''|*[!0-9]*) START_MS="$(( $(date +%s) * 1000 ))" ;;
    esac
fi

# _resp_num <response-file> <jq-expr> — a metric the response reported as a
# number, or "unknown". There is no path here that yields a default 0.
_resp_num() {
    _v="$(jq -r "($2) as \\$_m | if (\$_m | type) == \"number\" then (\$_m | tostring) else \"unknown\" end" "$1" 2>/dev/null)"
    case "${_v:-}" in
        ''|unknown) printf 'unknown' ;;
        *)          printf '%s' "$_v" ;;
    esac
}

# §17: the endpoint comes from the probe, never from a name. Source the same
# probe library discover-model-supply.sh uses and take the endpoint
# probe_runtimes() reports for the requested runtime. The probe endpoints are
# discovery URLs (/api/tags, /v1/models); the chat URL is derived from its
# base instead of hardcoding a per-runtime address.
_resolve_runtime_endpoint() {
    _probe_lib=""
    for _cand in "$SCRIPT_DIR/lib/model-supply-probe.sh" \
                 "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lib/model-supply-probe.sh"; do
        if [ -f "$_cand" ]; then _probe_lib="$_cand"; break; fi
    done
    if [ -z "$_probe_lib" ]; then
        _refuse 'probe library not found (lib/model-supply-probe.sh); no endpoint to dispatch to'
    fi
    # shellcheck source=scripts/lib/model-supply-probe.sh
    . "$_probe_lib"
    if ! command -v curl >/dev/null 2>&1; then
        _refuse 'curl is required to reach the runtime endpoint; refusing'
    fi
    # probe_runtimes references $CURL_TIMEOUT unguarded; the library is written
    # for callers that set it (discover-model-supply.sh does).
    CURL_TIMEOUT="${CURL_TIMEOUT:-5}"
    _runtimes="$(probe_runtimes)"
    _entry="$(printf '%s' "$_runtimes" | jq -c --arg n "$RUNTIME" \
        'first(.[] | select(.name == $n))')"
    if [ -z "$_entry" ]; then
        _refuse "runtime $RUNTIME not reported by the probe; no endpoint to dispatch to"
    fi
    _ep="$(printf '%s' "$_entry" | jq -r '.endpoint')"
    if [ "$(printf '%s' "$_entry" | jq -r '.reachable')" != "true" ]; then
        _refuse "runtime $RUNTIME endpoint unreachable at $_ep; the caller keeps its cloud tier"
    fi
    _base="${_ep%/*}"
    case "$_base" in
        */api) _base="${_base%/api}" ;;
    esac
    CHAT_URL="$_base/v1/chat/completions"
}

# _run_dispatch_http — one OpenAI-compatible chat completion through the
# probe-derived endpoint. Sets R_* from the response and returns curl's exit
# status; _HTTP_CODE carries the response status for the outcome mapping.
_run_dispatch_http() {
    _work="$(mktemp -d "${TMPDIR:-/tmp}/local-dispatch-http-XXXXXX")" || return 3
    _req="$_work/req.json"
    _resp="$_work/resp.json"
    if ! jq -n --arg model "$MODEL" --arg prompt "$(cat "$PROMPT_FILE")" \
        '{model: $model, messages: [{role: "user", content: $prompt}]}' > "$_req"; then
        rm -rf "$_work"
        return 3
    fi
    # 9>&-: the lock's file description must not survive into curl's children,
    # for the same reason as on the codex path.
    _w="$(timeout --preserve-status "$TIMEOUT_SECS" \
        curl -sS --max-time "$TIMEOUT_SECS" \
            -o "$_resp" \
            -w '%{time_starttransfer} %{time_total} %{http_code}' \
            -X POST "$CHAT_URL" -H 'Content-Type: application/json' \
            --data "@$_req" 9>&-)"
    _rc=$?
    _HTTP_CODE=""
    read -r _ttft_s _total_s _HTTP_CODE <<< "$_w"
    R_INPUT_TOKENS="unknown"
    R_OUTPUT_TOKENS="unknown"
    R_CACHED_TOKENS="unknown"
    R_PROMPT_TOK_S="unknown"
    R_DECODE_TOK_S="unknown"
    R_TTFT_MS="unknown"
    R_OUTPUT=""
    if jq -e 'type == "object"' "$_resp" >/dev/null 2>&1; then
        R_OUTPUT="$(jq -r '(.choices[0].message.content // "") | tostring' "$_resp" 2>/dev/null)"
        R_INPUT_TOKENS="$(_resp_num "$_resp" '.usage.prompt_tokens')"
        R_OUTPUT_TOKENS="$(_resp_num "$_resp" '.usage.completion_tokens')"
        R_CACHED_TOKENS="$(_resp_num "$_resp" '.usage.prompt_tokens_details.cached_tokens')"
    else
        R_OUTPUT="$(cat "$_resp" 2>/dev/null || true)"
    fi
    case "${_ttft_s:-}" in
        ''|.|*[!0-9.]*|*.*.*) : ;;
        *) R_TTFT_MS="$(awk -v s "$_ttft_s" 'BEGIN { printf "%d", s * 1000 + 0.5 }')" ;;
    esac
    if [ -n "${_ttft_s:-}" ] && [ -n "${_total_s:-}" ]; then
        # Decode window is generation after first byte. A server that buffers
        # the whole non-streaming body reports ttft ≈ total; then the whole
        # request window is the only measured window, and the rate is still a
        # measured ratio of observed counters — never a constant.
        _dec_s="$(awk -v t "$_total_s" -v f "$_ttft_s" \
            'BEGIN { d = t - f; if (d > 0) printf "%.3f", d; else if (t > 0) printf "%.3f", t; }')"
        if [ -n "$_dec_s" ] && [ "$R_OUTPUT_TOKENS" != "unknown" ]; then
            R_DECODE_TOK_S="$(awk -v n "$R_OUTPUT_TOKENS" -v s "$_dec_s" 'BEGIN { printf "%.2f", n / s }')"
        fi
        _pf_s="$(awk -v f "$_ttft_s" 'BEGIN { if (f > 0) printf "%.3f", f; }')"
        if [ -n "$_pf_s" ] && [ "$R_INPUT_TOKENS" != "unknown" ]; then
            R_PROMPT_TOK_S="$(awk -v n "$R_INPUT_TOKENS" -v s "$_pf_s" 'BEGIN { printf "%.2f", n / s }')"
        fi
    fi
    rm -rf "$_work"
    return "$_rc"
}

# stdout is the §16 envelope — the same key set as executor-dispatch.sh, so
# orchestration never special-cases which adapter ran. wall_clock_ms is the
# one metric this script measures itself, so it is the only one always numeric.
_emit() {
    # GNU `date +%s%3N` is unavailable on macOS, so prefer perl's Time::HiRes
    # and degrade to whole seconds, mirroring executor-dispatch.sh.
    _now="$(perl -MTime::HiRes -e 'printf("%.0f", Time::HiRes::time()*1000)' 2>/dev/null)"
    case "${_now:-}" in
        ''|*[!0-9]*) _now="$(( $(date +%s) * 1000 ))" ;;
    esac
    _wall=$(( _now - START_MS ))
    if [ "$_wall" -lt 0 ]; then _wall=0; fi
    jq -n \
        --arg schema        'autospec.dispatch-result.v1' \
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
}

_finish() { _emit "$1" "$2" "$3"; exit "$3"; }

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

if [ -n "$RUNTIME" ]; then
    SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd -P)" || SCRIPT_DIR=""
    _resolve_runtime_endpoint
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'POST %s\n' "$CHAT_URL"
        printf 'runtime=%s model=%s timeout_secs=%s cwd=%s prompt_file=%s\n' \
            "$RUNTIME" "$MODEL" "$TIMEOUT_SECS" "$WORKDIR" "$PROMPT_FILE"
        exit 0
    fi
else
    CODEX_ARGS="exec --oss --local-provider $PROVIDER --model $MODEL"

    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'codex %s\n' "$CODEX_ARGS"
        printf 'timeout_secs=%s cwd=%s prompt_file=%s\n' "$TIMEOUT_SECS" "$WORKDIR" "$PROMPT_FILE"
        exit 0
    fi
fi

if [ ! -d "$LOCK_DIR" ]; then mkdir -p "$LOCK_DIR"; fi
LOCK="$LOCK_DIR/local-model.lock"

_run_dispatch() {
    if [ -n "$RUNTIME" ]; then
        _run_dispatch_http
        return $?
    fi
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

if [ -n "$RUNTIME" ]; then
    # Runtime mode: the envelope IS the result, on success and failure alike.
    if [ "$_rc" -eq 124 ] || [ "$_rc" -eq 143 ]; then
        _finish timeout timeout 4
    elif [ "$_rc" -eq 0 ] && [[ "${_HTTP_CODE:-}" =~ ^2[0-9][0-9]$ ]]; then
        _finish success none 0
    else
        _finish failure harness_error 5
    fi
fi
if [ "$_rc" -eq 124 ] || [ "$_rc" -eq 143 ]; then
    printf 'local-dispatch: exceeded %ss ceiling\n' "$TIMEOUT_SECS" >&2
    exit 4
fi
exit "$_rc"
