#!/usr/bin/env sh
# uninstall.sh - remove the autospec-project skill from one or more harnesses.

set -eu

SKILL_NAME="autospec-project"
HARNESS=""
DRY_RUN=0

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|all] [--dry-run]
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

while [ $# -gt 0 ]; do
    case "$1" in
        --harness) shift; HARNESS="${1:-}" ;;
        --harness=*) HARNESS="${1#--harness=}" ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) err "unknown arg: $1"; usage; exit 2 ;;
    esac
    shift
done

[ -n "$HARNESS" ] || HARNESS=all
case "$HARNESS" in claude|opencode|codex|all) ;; *) err "invalid --harness: $HARNESS"; exit 2 ;; esac

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Claude Code:"
    remove_one "$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "OpenCode:"
    remove_one "$OPENCODE_DIR/agent/$SKILL_NAME.md"
fi
if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Codex CLI:"
    remove_one "$CODEX_DIR/prompts/$SKILL_NAME.md"
    remove_one "$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"
fi

info ""
[ "$DRY_RUN" -eq 1 ] && info "Dry run complete. No files were removed." || info "Done."
