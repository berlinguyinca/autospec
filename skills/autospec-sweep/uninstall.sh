#!/usr/bin/env sh
# uninstall.sh — remove the autospec-sweep skill from one or more harnesses.

set -eu

SKILL_NAME="autospec-sweep"
HARNESS=""
DRY_RUN=0

err() { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|all] [--dry-run]

Removes the ${SKILL_NAME} skill from the chosen harness.
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

remove_one "$HOME/.autospec/scripts/autospec-sweep-wizard.sh"
remove_one "$HOME/.autospec/scripts/autospec-sweep-run.sh"
remove_one "$HOME/.autospec/scripts/autospec-sweep-review.sh"
info "Done."
