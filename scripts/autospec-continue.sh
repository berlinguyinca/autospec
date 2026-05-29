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

usage() {
    cat <<'EOF'
Usage: autospec-continue.sh --from-message <path> [flags]

Required:
  --from-message <path>          Source assistant message file.

Flags:
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

while [ "$#" -gt 0 ]; do
    case "$1" in
        --from-message)  FROM_MESSAGE="${2:-}"; shift 2 ;;
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

# ── Step 5: handoff ───────────────────────────────────────────────
DISPATCHER=""
DISPATCHER_KIND=""
if command -v claude >/dev/null 2>&1; then
    DISPATCHER="$(command -v claude)"
    DISPATCHER_KIND="claude"
elif command -v autospec >/dev/null 2>&1; then
    DISPATCHER="$(command -v autospec)"
    DISPATCHER_KIND="autospec"
else
    echo "autospec-continue: no claude/autospec on PATH; final prompt follows on stdout" >&2
    printf '%s\n' "$FINAL_PROMPT"
    exit 0
fi

if ! _dispatcher_safe "$DISPATCHER"; then
    echo "autospec-continue: ERROR — refusing handoff: dispatcher in tmpdir: $DISPATCHER (set AUTOSPEC_HANDOFF_DISPATCHER=1 to override)" >&2
    exit 5
fi

echo "autospec-continue: handoff dispatcher=$DISPATCHER mode=$HANDOFF_MODE" >&2

RC=0
case "$DISPATCHER_KIND" in
    claude)
        if [ "$HANDOFF_MODE" = "interactive" ]; then
            "$DISPATCHER" "/autospec" "$FINAL_PROMPT" || RC=$?
        else
            "$DISPATCHER" "/autospec" "--autonomous" "$FINAL_PROMPT" || RC=$?
        fi
        ;;
    autospec)
        if [ "$HANDOFF_MODE" = "interactive" ]; then
            "$DISPATCHER" "$FINAL_PROMPT" || RC=$?
        else
            "$DISPATCHER" --autonomous "$FINAL_PROMPT" || RC=$?
        fi
        ;;
esac

exit "$RC"
