#!/usr/bin/env sh
# Remove autospec-compose-normalize from Claude Code, OpenCode, and Codex CLI.

set -eu

SKILL_NAME=autospec-compose-normalize
HARNESS=
DRY_RUN=0

err() { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    printf 'Usage: %s [--harness claude|opencode|codex|all] [--dry-run]\n' "$0"
}

remove_one() {
    target=$1
    if [ ! -e "$target" ] && [ ! -L "$target" ]; then
        info "  not installed: $target"
        return 0
    fi
    if [ "$DRY_RUN" -eq 1 ]; then
        info "  [dry-run] rm -f $target"
    else
        rm -f "$target"
        parent=$(dirname "$target")
        case "$parent" in
            */skills/$SKILL_NAME) rmdir "$parent" 2>/dev/null || true ;;
        esac
        info "  removed: $target"
    fi
}

while [ $# -gt 0 ]; do
    case "$1" in
        --harness)
            shift
            HARNESS=${1:-}
            ;;
        --harness=*) HARNESS=${1#--harness=} ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) err "unknown arg: $1"; usage; exit 2 ;;
    esac
    shift
done

if [ -z "$HARNESS" ]; then
    HARNESS=all
fi
case "$HARNESS" in
    claude|opencode|codex|all) ;;
    *) err "invalid --harness: $HARNESS"; exit 2 ;;
esac

CLAUDE_DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
OPENCODE_DIR=${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}
CODEX_DIR=${CODEX_HOME:-$HOME/.codex}

if [ "$HARNESS" = claude ] || [ "$HARNESS" = all ]; then
    info "Claude Code:"
    remove_one "$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = opencode ] || [ "$HARNESS" = all ]; then
    info "OpenCode:"
    remove_one "$OPENCODE_DIR/agent/$SKILL_NAME.md"
fi
if [ "$HARNESS" = codex ] || [ "$HARNESS" = all ]; then
    info "Codex CLI:"
    remove_one "$CODEX_DIR/prompts/$SKILL_NAME.md"
    remove_one "$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = all ]; then
    info "Shared workflow helper:"
    remove_one "$HOME/.autospec/scripts/autospec-compose-normalize-guard.sh"
fi

if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were removed."
else
    info "Done."
fi
