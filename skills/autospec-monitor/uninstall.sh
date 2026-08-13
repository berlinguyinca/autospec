#!/usr/bin/env sh
set -eu

SKILL_NAME=autospec-monitor
HARNESS=
DRY_RUN=0

usage() {
    printf 'Usage: %s [--harness claude|opencode|codex|all] [--dry-run]\n' "$0"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --harness) shift; HARNESS=${1:-} ;;
        --harness=*) HARNESS=${1#--harness=} ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'error: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

[ -n "$HARNESS" ] || HARNESS=all
case "$HARNESS" in claude|opencode|codex|all) ;; *) usage >&2; exit 2 ;; esac

remove_file() {
    target=$1
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '[dry-run] remove %s\n' "$target"
    else
        rm -f "$target"
    fi
}

CLAUDE_ROOT=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
OPENCODE_ROOT=${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}
CODEX_ROOT=${CODEX_HOME:-$HOME/.codex}

if [ "$HARNESS" = claude ] || [ "$HARNESS" = all ]; then
    remove_file "$CLAUDE_ROOT/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = opencode ] || [ "$HARNESS" = all ]; then
    remove_file "$OPENCODE_ROOT/agent/$SKILL_NAME.md"
fi
if [ "$HARNESS" = codex ] || [ "$HARNESS" = all ]; then
    remove_file "$CODEX_ROOT/prompts/$SKILL_NAME.md"
    remove_file "$CODEX_ROOT/skills/$SKILL_NAME/SKILL.md"
fi
printf 'uninstalled %s for %s\n' "$SKILL_NAME" "$HARNESS"
