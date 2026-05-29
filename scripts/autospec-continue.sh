#!/usr/bin/env bash
# scripts/autospec-continue.sh — autospec-continue orchestrator (issue #699).
#
# Wires the conversational-recommendation extraction helper (#702) into the
# /autospec-refine pipeline (#670, #693) and the final /autospec --autonomous
# handoff (#664). Operator can bypass refine, gate on confirmation, choose a
# lens mode, and source the conversational message from a file.
#
# Flags:
#   --from-message <path>          Read the source assistant message from <path>.
#   --no-loop, --once              Disable the default continuous loop and run
#                                  exactly one extract→refine→handoff pass
#                                  (legacy single-pass behavior). Also honored
#                                  via ~/.autospec/continue-no-loop.flag.
#   --skip-refine                  Bypass the refine step; hand off the
#                                  extracted block directly to /autospec.
#   --ask-confirm                  After (or instead of) refine, surface the
#                                  prompt and gate on operator response:
#                                    proceed       → continue handoff
#                                    cancel|abort  → exit 3 without handoff
#                                    interactive   → switch to interactive mode
#   --lens-mode deterministic|llm  Passed through to refine-prompt.sh.
#   --autonomous | --interactive   Handoff mode (default: --autonomous).
#   --artifact-dir <dir>           Override refinements artifact dir (test hook).
#   --repo-root <dir>              Override repo root (test hook).
#   --help                         Show usage and exit.
#
# Exit codes:
#   0  — extracted (and optionally refined) + handoff completed
#   2  — usage / bad args / source file missing
#   3  — operator cancelled at --ask-confirm gate, OR upstream extraction
#        returned 3 (empty recommendation / injection detected). Stderr of
#        the extractor is surfaced unmodified.
#   4  — empty refined prompt (refine returned exit 4)
#   5  — refused dispatcher path safety (claude/autospec under tmpdir)
#
# Dispatcher canonicalization (PR #693 pattern):
#   Resolves `claude`/`autospec` via `command -v` and rejects any binary
#   under /tmp/, /private/tmp/, /var/tmp/, /var/folders/, or $TMPDIR unless
#   AUTOSPEC_HANDOFF_DISPATCHER=1. Relative paths are always rejected.

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source the shared loop driver (issue #708) so /autospec-continue and
# /autospec-refine --continue + /autospec --loop share a single source of
# truth for the continuous-iteration loop. The single-pass /autospec-continue
# path is unchanged; the lib is sourced so the future --loop multi-iteration
# caller can invoke autospec_loop_run without re-implementing termination
# logic. See scripts/lib/autospec-loop.sh.
if [ -f "$SCRIPT_DIR/lib/autospec-loop.sh" ]; then
    # shellcheck source=lib/autospec-loop.sh
    . "$SCRIPT_DIR/lib/autospec-loop.sh"
fi

usage() {
    cat <<'EOF'
Usage: autospec-continue.sh --from-message <path> [flags]

Required:
  --from-message <path>          Source assistant message file.

Flags:
  --no-loop, --once              Disable continuous loop; run a single
                                 extract→refine→handoff pass (legacy
                                 behavior). Also honored via
                                 ~/.autospec/continue-no-loop.flag.
  --skip-refine                  Skip the refine step; hand off extracted
                                 block directly.
  --ask-confirm                  Gate on operator approval before handoff.
  --lens-mode deterministic|llm  Refine lens mode (default: deterministic).
  --autonomous                   Use `/autospec --autonomous` handoff (default).
  --interactive                  Use interactive `/autospec` handoff.
  --artifact-dir <dir>           Override refine artifact dir (test hook).
  --repo-root <dir>              Override repo root (test hook).
  --help                         Show this help.

Exit codes:
  0 ok | 2 usage | 3 cancelled or empty/injected recommendation
  4 empty refined prompt | 5 dispatcher path-safety refusal
EOF
}

FROM_MESSAGE=""
SKIP_REFINE=0
ASK_CONFIRM=0
LENS_MODE=""
HANDOFF_MODE="autonomous"
ARTIFACT_DIR=""
REPO_ROOT="."
NO_LOOP=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --from-message)  FROM_MESSAGE="${2:-}"; shift 2 ;;
        --no-loop|--once) NO_LOOP=1; shift ;;
        --skip-refine)   SKIP_REFINE=1; shift ;;
        --ask-confirm)   ASK_CONFIRM=1; shift ;;
        --lens-mode)     LENS_MODE="${2:-}"; shift 2 ;;
        --autonomous)    HANDOFF_MODE="autonomous"; shift ;;
        --interactive)   HANDOFF_MODE="interactive"; shift ;;
        --artifact-dir)  ARTIFACT_DIR="${2:-}"; shift 2 ;;
        --repo-root)     REPO_ROOT="${2:-}"; shift 2 ;;
        --help|-h)       usage; exit 0 ;;
        *)
            echo "autospec-continue: unknown arg: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ -z "$FROM_MESSAGE" ]; then
    echo "autospec-continue: --from-message <path> is required" >&2
    usage >&2
    exit 2
fi

if [ ! -f "$FROM_MESSAGE" ]; then
    echo "autospec-continue: source message not found: $FROM_MESSAGE" >&2
    exit 2
fi

if [ -n "$LENS_MODE" ]; then
    case "$LENS_MODE" in
        deterministic|llm) ;;
        *) echo "autospec-continue: --lens-mode must be deterministic|llm" >&2; exit 2 ;;
    esac
fi

EXTRACT_SH="$SCRIPT_DIR/extract-conversational-recommendation.sh"
REFINE_SH="$SCRIPT_DIR/refine-prompt.sh"

if [ ! -x "$EXTRACT_SH" ]; then
    echo "autospec-continue: extractor not executable: $EXTRACT_SH" >&2
    exit 2
fi

# ── Step 0: rate-limit bookkeeping (issue #700) ────────────────────
# Persist hashes + timestamps only (never prompt content) to
# ~/.autospec/continue-history.json. Operator may delete the file to reset.
# Override path via AUTOSPEC_CONTINUE_HISTORY for tests.
# Override clock via AUTOSPEC_CONTINUE_NOW (epoch seconds) for tests.
_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    else
        echo "autospec-continue: no sha256sum/shasum on PATH" >&2
        return 1
    fi
}

HISTORY_FILE="${AUTOSPEC_CONTINUE_HISTORY:-$HOME/.autospec/continue-history.json}"
mkdir -p "$(dirname "$HISTORY_FILE")" 2>/dev/null || true

NOW="${AUTOSPEC_CONTINUE_NOW:-$(date +%s)}"
SOURCE_HASH="$(_sha256 < "$FROM_MESSAGE")"
[ -n "$SOURCE_HASH" ] || { echo "autospec-continue: source hash failed" >&2; exit 2; }

# Read existing entries (jq optional; fall back to grep parsing).
DUPLICATE_WINDOW=60        # seconds
OSCILLATION_WINDOW=3600    # 60 minutes
OSCILLATION_THRESHOLD=3

if [ -f "$HISTORY_FILE" ]; then
    if command -v jq >/dev/null 2>&1; then
        DUP_COUNT="$(jq --arg h "$SOURCE_HASH" --argjson now "$NOW" --argjson w "$DUPLICATE_WINDOW" \
            '[.entries[]? | select(.source_message_hash == $h) | select(($now - .timestamp) < $w)] | length' \
            "$HISTORY_FILE" 2>/dev/null || echo 0)"
    else
        DUP_COUNT=0
    fi
    if [ "${DUP_COUNT:-0}" -gt 0 ]; then
        echo "code_health:continue_recent_duplicate" >&2
        echo "autospec-continue: same source message seen within ${DUPLICATE_WINDOW}s; refusing to re-run" >&2
        exit 3
    fi
fi

# ── Step 1: extract conversational recommendation ─────────────────
EXTRACTED=""
EXTRACT_RC=0
EXTRACTED="$(bash "$EXTRACT_SH" --message "$FROM_MESSAGE")" || EXTRACT_RC=$?
if [ "$EXTRACT_RC" -ne 0 ]; then
    # Extractor already logged the code_health:* category to its stderr.
    exit "$EXTRACT_RC"
fi

if [ -z "$EXTRACTED" ]; then
    echo "autospec-continue: extractor returned empty output" >&2
    exit 3
fi

EXTRACTED_HASH="$(printf '%s' "$EXTRACTED" | _sha256)"
[ -n "$EXTRACTED_HASH" ] || { echo "autospec-continue: extracted hash failed" >&2; exit 2; }

# Oscillation check: same extracted prompt appearing ≥3x in last 60 min.
if [ -f "$HISTORY_FILE" ] && command -v jq >/dev/null 2>&1; then
    OSC_COUNT="$(jq --arg h "$EXTRACTED_HASH" --argjson now "$NOW" --argjson w "$OSCILLATION_WINDOW" \
        '[.entries[]? | select(.extracted_prompt_hash == $h) | select(($now - .timestamp) < $w)] | length' \
        "$HISTORY_FILE" 2>/dev/null || echo 0)"
    # Count prior occurrences; current invocation makes it +1, so the
    # threshold trips when prior count >= threshold-1.
    if [ "${OSC_COUNT:-0}" -ge $((OSCILLATION_THRESHOLD - 1)) ]; then
        echo "code_health:continue_oscillation" >&2
        echo "autospec-continue: same extracted prompt seen ${OSC_COUNT}x in last ${OSCILLATION_WINDOW}s; refusing to oscillate" >&2
        exit 3
    fi
fi

# Atomic append of new entry. Schema: { "entries": [ {timestamp, source_message_hash, extracted_prompt_hash}, ... ] }
HISTORY_TMP="${HISTORY_FILE}.tmp.$$"
if command -v jq >/dev/null 2>&1; then
    if [ -f "$HISTORY_FILE" ]; then
        jq --argjson now "$NOW" --arg s "$SOURCE_HASH" --arg e "$EXTRACTED_HASH" \
            '.entries = ((.entries // []) + [{timestamp: $now, source_message_hash: $s, extracted_prompt_hash: $e}])' \
            "$HISTORY_FILE" > "$HISTORY_TMP" 2>/dev/null \
            || printf '{"entries":[{"timestamp":%s,"source_message_hash":"%s","extracted_prompt_hash":"%s"}]}\n' \
                "$NOW" "$SOURCE_HASH" "$EXTRACTED_HASH" > "$HISTORY_TMP"
    else
        printf '{"entries":[{"timestamp":%s,"source_message_hash":"%s","extracted_prompt_hash":"%s"}]}\n' \
            "$NOW" "$SOURCE_HASH" "$EXTRACTED_HASH" > "$HISTORY_TMP"
    fi
    mv "$HISTORY_TMP" "$HISTORY_FILE"
fi

# ── Step 1.5: continuous loop mode (issue #710) ────────────────────
# Default behavior: invoke the shared loop driver from
# scripts/lib/autospec-loop.sh (#708) — extract → refine → handoff →
# harvest → re-extract → loop until convergence/oscillation/stop/cap.
# Opt-outs: --no-loop / --once flag, or ~/.autospec/continue-no-loop.flag.
LOOP_FLAG_FILE="${HOME}/.autospec/continue-no-loop.flag"
if [ -f "$LOOP_FLAG_FILE" ]; then
    NO_LOOP=1
fi

if [ "$NO_LOOP" -eq 0 ] && declare -F autospec_loop_run >/dev/null 2>&1; then
    # Resolve the refine-prompt.sh script path the shared driver dispatches
    # each iteration to.
    SCRIPT_PATH="$REFINE_SH"
    PROMPT="$EXTRACTED"
    ARTIFACT_DIR="${ARTIFACT_DIR:-.autospec/refinements}"
    MEMORY_ROOT="${MEMORY_ROOT:-$REPO_ROOT/.autospec/memory}"
    ROUNDS="${ROUNDS:-3}"
    MAX_ITERATIONS="${AUTOSPEC_LOOP_MAX_ITERATIONS:-${MAX_ITERATIONS:-5}}"
    TOKEN_CAP="${AUTOSPEC_LOOP_TOKEN_CAP:-2000000}"
    TIME_CAP="${AUTOSPEC_LOOP_TIME_CAP:-21600}"
    mkdir -p "$ARTIFACT_DIR" 2>/dev/null || true
    echo "autospec-continue: entering continuous loop mode (--no-loop to opt out)" >&2
    autospec_loop_run
    exit $?
fi

# ── Step 2: refine (unless --skip-refine) ─────────────────────────
FINAL_PROMPT="$EXTRACTED"

if [ "$SKIP_REFINE" -eq 0 ]; then
    if [ ! -x "$REFINE_SH" ]; then
        echo "autospec-continue: refine-prompt.sh not executable: $REFINE_SH" >&2
        exit 2
    fi
    # Run refine in --dry-run so it does NOT hand off itself — we own the
    # handoff path below so --ask-confirm and dispatcher-safety land in one
    # place. Write the refined prompt to a sibling file we read back.
    REFINED_OUT="$(mktemp -t autospec-continue-refined.XXXXXX)"
    REFINE_ARTIFACT_DIR="${ARTIFACT_DIR:-.autospec/refinements}"
    REFINE_ARGS=( "$EXTRACTED" --dry-run --output "$REFINED_OUT" \
        --artifact-dir "$REFINE_ARTIFACT_DIR" --repo-root "$REPO_ROOT" )
    if [ -n "$LENS_MODE" ]; then
        REFINE_ARGS+=( --lens-mode "$LENS_MODE" )
    fi
    REFINE_RC=0
    bash "$REFINE_SH" "${REFINE_ARGS[@]}" >/dev/null 2>&1 || REFINE_RC=$?
    if [ "$REFINE_RC" -ne 0 ]; then
        echo "autospec-continue: refine failed rc=$REFINE_RC" >&2
        rm -f "$REFINED_OUT" 2>/dev/null || true
        exit "$REFINE_RC"
    fi
    if [ ! -s "$REFINED_OUT" ]; then
        echo "autospec-continue: refine produced empty output" >&2
        rm -f "$REFINED_OUT" 2>/dev/null || true
        exit 4
    fi
    FINAL_PROMPT="$(cat "$REFINED_OUT")"
    rm -f "$REFINED_OUT" 2>/dev/null || true
fi

# ── Step 3: --ask-confirm gate ────────────────────────────────────
if [ "$ASK_CONFIRM" -eq 1 ]; then
    {
        printf '\n=== autospec-continue: review prompt ===\n'
        printf '%s\n' "$FINAL_PROMPT"
        printf '=== end prompt ===\n'
        printf 'Reply [proceed|cancel|interactive]: '
    } >&2
    REPLY=""
    if ! IFS= read -r REPLY; then
        echo "autospec-continue: no operator response; aborting" >&2
        exit 3
    fi
    case "$(printf '%s' "$REPLY" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')" in
        proceed|yes|y|ok) ;;
        interactive)      HANDOFF_MODE="interactive" ;;
        cancel|abort|no|n|"")
            echo "autospec-continue: cancelled by operator" >&2
            exit 3
            ;;
        *)
            echo "autospec-continue: unrecognised response '$REPLY'; aborting" >&2
            exit 3
            ;;
    esac
fi

# ── Step 4: dispatcher resolution + canonicalization (PR #693) ────
_canonicalize() {
    local p="$1"
    local r=""
    if command -v realpath >/dev/null 2>&1; then
        r="$(realpath -m "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
        r="$(realpath "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    if command -v readlink >/dev/null 2>&1; then
        r="$(readlink -f "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
        if [ -L "$p" ]; then
            local target
            target="$(readlink "$p" 2>/dev/null)" || target=""
            if [ -n "$target" ]; then
                case "$target" in
                    /*) printf '%s' "$target"; return ;;
                    *)  printf '%s/%s' "$(dirname "$p")" "$target"; return ;;
                esac
            fi
        fi
    fi
    if command -v python3 >/dev/null 2>&1; then
        r="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    printf '%s' "$p"
}

_dispatcher_safe() {
    local resolved="$1"
    [ -n "$resolved" ] || return 1
    if [ -n "${AUTOSPEC_HANDOFF_DISPATCHER:-}" ]; then
        case "$resolved" in /*) return 0 ;; *) return 1 ;; esac
    fi
    case "$resolved" in
        /*) ;;
        *) return 1 ;;
    esac
    case "$resolved" in
        /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*) return 1 ;;
    esac
    if [ -n "${TMPDIR:-}" ]; then
        case "$resolved" in
            "$TMPDIR"*|"${TMPDIR%/}"/*) return 1 ;;
        esac
    fi
    local canon
    canon="$(_canonicalize "$resolved")"
    if [ -n "$canon" ] && [ "$canon" != "$resolved" ]; then
        case "$canon" in
            /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*) return 1 ;;
        esac
        if [ -n "${TMPDIR:-}" ]; then
            case "$canon" in
                "$TMPDIR"*|"${TMPDIR%/}"/*) return 1 ;;
            esac
        fi
    fi
    return 0
}

# ── Step 5: harness-aware handoff (issue #723) ────────────────────
# Delegates to scripts/lib/autospec-harness-detect.sh so Codex CLI gets
# `codex exec --skip-git-repo-check "/autospec --autonomous $PROMPT"` and
# OpenCode gets `opencode "/autospec" "--autonomous" "$PROMPT"`.
HARNESS_LIB="$SCRIPT_DIR/lib/autospec-harness-detect.sh"
if [ -f "$HARNESS_LIB" ]; then
    # shellcheck source=lib/autospec-harness-detect.sh
    . "$HARNESS_LIB"
fi

if declare -F autospec_harness_resolve_dispatcher >/dev/null 2>&1; then
    _resolve_rc=0
    _resolve_err="$(mktemp -t autospec-continue-resolve.XXXXXX)"
    ( autospec_harness_resolve_dispatcher ) >/dev/null 2>"$_resolve_err" || _resolve_rc=$?
    if [ "$_resolve_rc" = 0 ]; then
        autospec_harness_resolve_dispatcher
        rm -f "$_resolve_err"
        echo "autospec-continue: handoff harness=$AUTOSPEC_HARNESS_KIND dispatcher=$AUTOSPEC_HARNESS_DISPATCHER mode=$HANDOFF_MODE" >&2
        autospec_harness_invoke "$HANDOFF_MODE" "$FINAL_PROMPT"
        exit $?
    fi
    if [ "$_resolve_rc" = 5 ]; then
        cat "$_resolve_err" >&2
        rm -f "$_resolve_err"
        exit 5
    fi
    rm -f "$_resolve_err"
fi

# Legacy `autospec` binary fallback (preserved per issue #723 backward-compat).
if command -v autospec >/dev/null 2>&1; then
    DISPATCHER="$(command -v autospec)"
    if ! _dispatcher_safe "$DISPATCHER"; then
        echo "autospec-continue: ERROR — refusing handoff: dispatcher in tmpdir: $DISPATCHER (set AUTOSPEC_HANDOFF_DISPATCHER=1 to override)" >&2
        exit 5
    fi
    echo "autospec-continue: handoff dispatcher=$DISPATCHER mode=$HANDOFF_MODE (legacy autospec binary)" >&2
    RC=0
    if [ "$HANDOFF_MODE" = "interactive" ]; then
        "$DISPATCHER" "$FINAL_PROMPT" || RC=$?
    else
        "$DISPATCHER" --autonomous "$FINAL_PROMPT" || RC=$?
    fi
    exit "$RC"
fi

echo "autospec-continue: no harness dispatcher on PATH; final prompt follows on stdout" >&2
printf '%s\n' "$FINAL_PROMPT"
exit 0
