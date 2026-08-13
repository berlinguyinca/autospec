#!/usr/bin/env bash
# scripts/lib/autospec-harness-detect.sh — shared harness-aware loop dispatcher
# resolver (issue #723).
#
# Single source of truth for "which AI harness am I running under, and what is
# the canonical handoff invocation for `/autospec --autonomous <prompt>` on
# that harness?". Sourced by:
#   - scripts/refine-prompt.sh         (run_handoff)
#   - scripts/autospec-continue.sh     (Step 5 dispatch)
#   - scripts/qa-heal-loop.sh          (AUTOSPEC_RUN_CMD fallback dispatcher)
#   - scripts/refine-prompt-lens-llm.sh (LLM dispatcher resolution alignment)
#
# Detection order:
#   1. AUTOSPEC_HANDOFF_DISPATCHER_KIND env override (claude|codex|opencode).
#   2. Active harness runtime marker.
#   3. Skill-mount probe:
#        ~/.claude/skills/...               → Claude Code
#        ~/.codex/prompts/... | ~/.codex/   → Codex CLI
#        ~/.config/opencode/agent/...       → OpenCode
#   3. Binary-on-PATH probe (claude > codex > opencode).
#
# Per-harness invocation table (issue #723):
#   Claude Code → claude "/autospec" "--autonomous" "$PROMPT"
#   Codex CLI   → codex exec --skip-git-repo-check "/autospec --autonomous $PROMPT"
#   OpenCode    → opencode "/autospec" "--autonomous" "$PROMPT"
#
# Path-safety (mirrors PR #693): rejects relative paths and any binary under
# /tmp, /private/tmp, /var/tmp, /var/folders, or $TMPDIR, before AND after
# canonicalization via realpath. `AUTOSPEC_HANDOFF_DISPATCHER=1` overrides
# the tmpdir denylist but still requires an absolute path.
#
# Missing harness binary surfaces as:
#   code_health:loop_handoff_no_dispatcher_for_harness harness=<kind>
# and `autospec_harness_resolve_dispatcher` returns exit 3.

# Guard double-source.
if [ -n "${_AUTOSPEC_HARNESS_DETECT_LOADED:-}" ]; then
    return 0 2>/dev/null || true
fi
_AUTOSPEC_HARNESS_DETECT_LOADED=1

_autospec_harness_table() {
    if [ -n "${AUTOSPEC_HARNESS_RUNTIME_ALIASES:-}" ]; then
        printf '%s' "$AUTOSPEC_HARNESS_RUNTIME_ALIASES"
        return
    fi
    local script_dir
    script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
    for candidate in \
        "${AUTOSPEC_CONFIG_DIR:-$HOME/.autospec/config}/harness-runtime-aliases.tsv" \
        "$script_dir/../../config/harness-runtime-aliases.tsv" \
        "$script_dir/harness-runtime-aliases.tsv"; do
        if [ -f "$candidate" ]; then printf '%s' "$candidate"; return; fi
    done
    return 1
}

autospec_harness_supported_ids() {
    local table
    table="$(_autospec_harness_table)" || return 1
    awk -F '\t' 'NF == 4 { print $1 }' "$table"
}

# ── canonicalize (lifted from refine-prompt.sh / autospec-continue.sh) ──
_autospec_harness_canonicalize() {
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

# autospec_harness_dispatcher_safe <resolved-path>
# Returns 0 if safe, 1 otherwise. Mirrors PR #693 / #692 rules.
autospec_harness_dispatcher_safe() {
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
    canon="$(_autospec_harness_canonicalize "$resolved")"
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

# autospec_harness_detect
# Probes the environment to determine the active harness. Emits one of:
#   claude | codex | opencode
# on stdout. Detection order: AUTOSPEC_HANDOFF_DISPATCHER_KIND env override,
# then skill-mount probe (test hook: AUTOSPEC_HARNESS_PROBE_ROOT overrides
# ${HOME} for fixtures), then binary-on-PATH fallback. Defaults to "claude"
# when nothing is detected so existing behavior is preserved.
autospec_harness_detect() {
    if [ -n "${AUTOSPEC_HANDOFF_DISPATCHER_KIND:-}" ]; then
        if autospec_harness_binary_for "$AUTOSPEC_HANDOFF_DISPATCHER_KIND" >/dev/null; then
            printf '%s' "$AUTOSPEC_HANDOFF_DISPATCHER_KIND"
            return 0
        fi
    fi
    # Installed skill homes describe what can run, not which harness owns this
    # process. Prefer runtime markers before probing homes so a Codex session on
    # a machine that also has Claude installed does not silently dispatch to
    # Claude. Explicit override above remains authoritative.
    if [ -n "${CODEX_THREAD_ID:-}" ] || [ -n "${CODEX_CI:-}" ]; then
        printf 'codex'; return 0
    fi
    if [ -n "${CLAUDECODE:-}" ] || [ -n "${CLAUDE_CODE_ENTRYPOINT:-}" ]; then
        printf 'claude'; return 0
    fi
    if [ -n "${OPENCODE:-}" ] || [ -n "${OPENCODE_SESSION_ID:-}" ]; then
        printf 'opencode'; return 0
    fi
    local probe_root="${AUTOSPEC_HARNESS_PROBE_ROOT:-$HOME}"
    if [ -d "$probe_root/.claude/skills" ]; then
        printf 'claude'; return 0
    fi
    if [ -d "$probe_root/.codex/prompts" ] || [ -d "$probe_root/.codex" ]; then
        printf 'codex'; return 0
    fi
    if [ -d "$probe_root/.config/opencode/agent" ] || [ -d "$probe_root/.config/opencode" ]; then
        printf 'opencode'; return 0
    fi
    local kind binary
    while IFS= read -r kind; do
        binary="$(autospec_harness_binary_for "$kind")" || continue
        if command -v "$binary" >/dev/null 2>&1; then printf '%s' "$kind"; return 0; fi
    done <<EOF
$(autospec_harness_supported_ids)
EOF
    # Default fallback so callers don't NPE; the binary-resolve step below
    # will exit 3 when no dispatcher binary is present.
    printf 'claude'
    return 0
}

# autospec_harness_binary_for <kind>
# Emits the canonical binary name for the harness on stdout.
autospec_harness_binary_for() {
    local table
    table="$(_autospec_harness_table)" || return 1
    awk -F '\t' -v id="$1" '$1 == id { print $2; found=1; exit } END { exit found ? 0 : 1 }' "$table"
}

# autospec_harness_resolve_dispatcher
# Sets globals: AUTOSPEC_HARNESS_KIND, AUTOSPEC_HARNESS_DISPATCHER (absolute
# canonical path to the binary). Exits 3 with the named code_health category
# if the binary for the detected harness is not on PATH.
autospec_harness_resolve_dispatcher() {
    local kind
    kind="$(autospec_harness_detect)"
    local bin
    bin="$(autospec_harness_binary_for "$kind")" || {
        echo "code_health:loop_handoff_no_dispatcher_for_harness harness=$kind" >&2
        exit 3
    }
    local resolved=""
    if command -v "$bin" >/dev/null 2>&1; then
        resolved="$(command -v "$bin")"
    fi
    if [ -z "$resolved" ]; then
        echo "code_health:loop_handoff_no_dispatcher_for_harness harness=$kind binary=$bin" >&2
        echo "autospec-harness: detected harness=$kind but '$bin' not on PATH; set AUTOSPEC_HANDOFF_DISPATCHER_KIND to override" >&2
        exit 3
    fi
    if ! autospec_harness_dispatcher_safe "$resolved"; then
        echo "autospec-harness: ERROR — refusing handoff: dispatcher in tmpdir: $resolved (set AUTOSPEC_HANDOFF_DISPATCHER=1 to override)" >&2
        exit 5
    fi
    AUTOSPEC_HARNESS_KIND="$kind"
    AUTOSPEC_HARNESS_DISPATCHER="$resolved"
    export AUTOSPEC_HARNESS_KIND AUTOSPEC_HARNESS_DISPATCHER
    return 0
}

# autospec_harness_invoke <handoff-mode: autonomous|interactive> <prompt>
# Invokes /autospec on the resolved harness using the per-harness invocation
# form documented in issue #723. Requires autospec_harness_resolve_dispatcher
# to have been called first (or sets globals on its own).
autospec_harness_invoke() {
    local mode="$1"; shift
    local prompt="$1"; shift || true
    if [ -z "${AUTOSPEC_HARNESS_DISPATCHER:-}" ] || [ -z "${AUTOSPEC_HARNESS_KIND:-}" ]; then
        autospec_harness_resolve_dispatcher || return $?
    fi
    case "$AUTOSPEC_HARNESS_KIND" in
        claude)
            if [ "$mode" = "interactive" ]; then
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec" "$prompt"
            else
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec" "--autonomous" "$prompt"
            fi
            return $?
            ;;
        codex)
            # Codex CLI takes the full slash-command + args as a single string.
            if [ "$mode" = "interactive" ]; then
                "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec $prompt"
            else
                "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec --autonomous $prompt"
            fi
            return $?
            ;;
        opencode)
            if [ "$mode" = "interactive" ]; then
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec" "$prompt"
            else
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec" "--autonomous" "$prompt"
            fi
            return $?
            ;;
        *)
            echo "code_health:loop_handoff_no_dispatcher_for_harness harness=$AUTOSPEC_HARNESS_KIND" >&2
            return 3
            ;;
    esac
}
