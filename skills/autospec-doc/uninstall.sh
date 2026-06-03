#!/usr/bin/env sh
# uninstall.sh — remove the autospec skill from one or more
# agent harnesses (Claude Code, OpenCode, Codex CLI).
#
# Usage:
#   ./uninstall.sh                     # interactive — prompts for harness
#   ./uninstall.sh --harness <name>    # one of: claude | opencode | codex | all
#   ./uninstall.sh --dry-run           # print what would be done; do nothing
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)
#
# Idempotent: removing an already-uninstalled file is a no-op.

set -eu

SKILL_NAME="autospec-doc"

HARNESS=""
DRY_RUN=0

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|all] [--dry-run]

Removes the ${SKILL_NAME} skill from the chosen harness's standard
skill/agent/prompt directory.
EOF
}

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        eval "$@"
    fi
}

remove_one() {
    target="$1"

    if [ -e "$target" ] || [ -L "$target" ]; then
        run "rm -f \"$target\""
        info "  removed: $target"
        # Try to remove an empty parent skill directory (Claude only — its layout
        # has a per-skill subdir).
        parent="$(dirname "$target")"
        case "$parent" in
            */skills/"$SKILL_NAME")
                if [ "$DRY_RUN" -eq 0 ] && [ -d "$parent" ]; then
                    rmdir "$parent" 2>/dev/null || true
                fi
                ;;
        esac
    else
        info "  not installed: $target"
    fi
}

# ---------- arg parse ------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --harness)
            shift
            HARNESS="${1:-}"
            ;;
        --harness=*)
            HARNESS="${1#--harness=}"
            ;;
        --dry-run)
            DRY_RUN=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "unknown arg: $1"
            usage
            exit 2
            ;;
    esac
    shift
done

if [ -z "$HARNESS" ]; then
    if [ -t 0 ] && [ -t 1 ]; then
        info "Which harness should we uninstall from?"
        info "  1) claude    (Claude Code)"
        info "  2) opencode  (OpenCode)"
        info "  3) codex     (Codex CLI)"
        info "  4) all"
        printf 'choice [4]: '
        read -r choice || choice=""
        case "${choice:-4}" in
            1) HARNESS=claude ;;
            2) HARNESS=opencode ;;
            3) HARNESS=codex ;;
            4|"") HARNESS=all ;;
            claude|opencode|codex|all) HARNESS="$choice" ;;
            *) err "invalid choice: $choice"; exit 2 ;;
        esac
    else
        HARNESS=all
    fi
fi

case "$HARNESS" in
    claude|opencode|codex|all) ;;
    *) err "invalid --harness: $HARNESS"; usage; exit 2 ;;
esac

# ---------- per-harness paths ---------------------------------------------

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"

CLAUDE_DEST="$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
OPENCODE_DEST="$OPENCODE_DIR/agent/$SKILL_NAME.md"
CODEX_DEST="$CODEX_DIR/prompts/$SKILL_NAME.md"

# ---------- remove ---------------------------------------------------------

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Claude Code:"
    remove_one "$CLAUDE_DEST"
fi

if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "OpenCode:"
    remove_one "$OPENCODE_DEST"
fi

if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Codex CLI:"
    remove_one "$CODEX_DEST"
fi

info ""
if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were removed."
else
    info "Done."
fi

exit 0
