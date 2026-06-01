#!/usr/bin/env bash
# verify-harness-setup.sh — Cross-harness setup doctor for autospec.
#
# Ensures every supported harness (Claude Code, Codex CLI, OpenCode) can find
# the same persistent memory + project guidance. Idempotent; safe to re-run.
#
# Wired into: /autospec-sweep init (and runnable standalone as a doctor).
#
# What it checks / fixes:
#   1. Memory plumbing — delegates to auto-init-memory.sh
#        (migrates ~/.claude/projects/<slug>/memory → docs/memory/, symlinks
#         back, appends Memory inventory block to AGENTS.md).
#   2. AGENTS.md — exists at repo root (Codex CLI + OpenCode read this).
#   3. CLAUDE.md — exists at repo root. If missing, writes a one-line pointer
#      to AGENTS.md so all three harnesses converge on the same guidance.
#   4. docs/memory/MEMORY.md — index file exists (entry point for auto-memory).
#   5. ~/.claude/projects/<slug>/memory symlink resolves to docs/memory/.
#
# Usage:
#   verify-harness-setup.sh            # fix-mode (default): apply fixes, print report
#   verify-harness-setup.sh --check    # report-only: never write
#   verify-harness-setup.sh --quiet    # fix-mode, suppress OK lines
#
# Exit codes:
#   0  all checks pass (or were fixed)
#   1  unfixable problem (only in --check mode, or fix attempt failed)

set +e

MODE="fix"
QUIET=0
for arg in "$@"; do
    case "$arg" in
        --check) MODE="check" ;;
        --quiet) QUIET=1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$REPO_ROOT" ]; then
    echo "verify-harness-setup: not inside a git repo — skipping" >&2
    exit 0
fi

CC_SLUG=$(echo "$REPO_ROOT" | tr '/' '-')
CC_PATH="$HOME/.claude/projects/${CC_SLUG}/memory"
REPO_MEMORY="$REPO_ROOT/docs/memory"
AGENTS_MD="$REPO_ROOT/AGENTS.md"
CLAUDE_MD="$REPO_ROOT/CLAUDE.md"

EXIT=0
declare -a REPORT

_report() {
    local status="$1" component="$2" detail="$3"
    if [ "$QUIET" = "1" ] && [ "$status" = "OK" ]; then return 0; fi
    REPORT+=("$(printf '  [%s] %-22s %s' "$status" "$component" "$detail")")
}

# ── 1. Memory plumbing ────────────────────────────────────────────────────────
if [ "$MODE" = "fix" ]; then
    if [ -x "$AUTOSPEC_SCRIPTS_DIR/auto-init-memory.sh" ]; then
        bash "$AUTOSPEC_SCRIPTS_DIR/auto-init-memory.sh"
    else
        _report "FAIL" "memory-init" "auto-init-memory.sh not found at $AUTOSPEC_SCRIPTS_DIR"
        EXIT=1
    fi
fi

# ── 2. docs/memory/ + MEMORY.md ───────────────────────────────────────────────
if [ -d "$REPO_MEMORY" ]; then
    if [ -f "$REPO_MEMORY/MEMORY.md" ]; then
        _report "OK" "docs/memory" "index present"
    else
        if [ "$MODE" = "fix" ]; then
            printf '# Memory index\n' > "$REPO_MEMORY/MEMORY.md"
            _report "FIX" "docs/memory" "created empty MEMORY.md index"
        else
            _report "FAIL" "docs/memory" "MEMORY.md index missing"
            EXIT=1
        fi
    fi
else
    _report "FAIL" "docs/memory" "directory missing (auto-init-memory.sh failed?)"
    EXIT=1
fi

# ── 3. Claude Code symlink ────────────────────────────────────────────────────
if [ -L "$CC_PATH" ] && [ "$(readlink "$CC_PATH")" = "$REPO_MEMORY" ]; then
    _report "OK" "claude-code" "memory symlink → docs/memory/"
elif [ -e "$CC_PATH" ]; then
    _report "FAIL" "claude-code" "$CC_PATH exists but is not the expected symlink"
    EXIT=1
else
    _report "FAIL" "claude-code" "memory symlink missing (expected $CC_PATH)"
    EXIT=1
fi

# ── 4. AGENTS.md (Codex CLI + OpenCode entrypoint) ────────────────────────────
if [ -f "$AGENTS_MD" ]; then
    if grep -q "^## Memory inventory" "$AGENTS_MD" 2>/dev/null; then
        _report "OK" "codex+opencode" "AGENTS.md has Memory inventory block"
    else
        _report "FAIL" "codex+opencode" "AGENTS.md missing Memory inventory section"
        EXIT=1
    fi
else
    _report "FAIL" "codex+opencode" "AGENTS.md missing at repo root"
    EXIT=1
fi

# ── 5. CLAUDE.md (cross-reference to AGENTS.md) ───────────────────────────────
if [ -f "$CLAUDE_MD" ]; then
    _report "OK" "claude-code" "CLAUDE.md present"
elif [ "$MODE" = "fix" ] && [ -f "$AGENTS_MD" ]; then
    cat > "$CLAUDE_MD" <<'CLAUDE_STUB'
# Claude Code project guidance

This project uses a single source of truth at [`AGENTS.md`](AGENTS.md), which
is also read by Codex CLI and OpenCode. Read that file first.

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/);
the index is [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).
CLAUDE_STUB
    _report "FIX" "claude-code" "created CLAUDE.md pointer to AGENTS.md"
else
    _report "FAIL" "claude-code" "CLAUDE.md missing at repo root"
    EXIT=1
fi

# ── Print report ──────────────────────────────────────────────────────────────
if [ "${#REPORT[@]}" -gt 0 ]; then
    echo "verify-harness-setup ($MODE):"
    for line in "${REPORT[@]}"; do
        echo "$line"
    done
fi

exit "$EXIT"
