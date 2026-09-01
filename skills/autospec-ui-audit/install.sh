#!/usr/bin/env sh
set -eu

SKILL_NAME=autospec-ui-audit
RAW_BASE="${AUTOSPEC_UI_AUDIT_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_UI_AUDIT_REF:-main}/skills/$SKILL_NAME}"
SCRIPT_PATH=${0:-}
if [ -n "$SCRIPT_PATH" ] && [ -f "$SCRIPT_PATH" ]; then
    SKILL_DIR=$(CDPATH= cd -- "$(dirname "$SCRIPT_PATH")" && pwd)
else
    SKILL_DIR=
fi
HARNESS=
DRY_RUN=0
UPDATE=0
TMP_DIR=

usage() {
    printf 'Usage: %s [--harness claude|opencode|codex|pi|all] [--update] [--dry-run]\n' "$0"
}

cleanup() {
    [ -z "${TMP_DIR:-}" ] || rm -rf "$TMP_DIR"
}
trap cleanup EXIT HUP INT TERM

while [ $# -gt 0 ]; do
    case "$1" in
        --harness) shift; HARNESS=${1:-} ;;
        --harness=*) HARNESS=${1#--harness=} ;;
        --update) UPDATE=1 ;;
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'error: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

[ -n "$HARNESS" ] || HARNESS=all
case "$HARNESS" in claude|opencode|codex|all) ;; *) usage >&2; exit 2 ;; esac

if [ -z "$SKILL_DIR" ]; then
    command -v curl >/dev/null 2>&1 || { printf 'error: curl is required\n' >&2; exit 1; }
    TMP_DIR=$(mktemp -d)
    SKILL_DIR=$TMP_DIR
    mkdir -p "$SKILL_DIR/codex" "$SKILL_DIR/opencode" "$SKILL_DIR/scripts"
    for rel in SKILL.md codex/prompt.md opencode/agent.md scripts/route-inventory.mjs; do
        curl -fsSL "$RAW_BASE/$rel" -o "$SKILL_DIR/$rel"
    done
fi

copy_file() {
    source=$1
    target=$2
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '[dry-run] install %s\n' "$target"
    else
        mkdir -p "$(dirname "$target")"
        cp "$source" "$target"
    fi
}

CLAUDE_ROOT=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
OPENCODE_ROOT=${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}
CODEX_ROOT=${CODEX_HOME:-$HOME/.codex}
SCRIPTS_ROOT=${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}

if [ "$HARNESS" = claude ] || [ "$HARNESS" = all ]; then
    copy_file "$SKILL_DIR/SKILL.md" "$CLAUDE_ROOT/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = opencode ] || [ "$HARNESS" = all ]; then
    copy_file "$SKILL_DIR/opencode/agent.md" "$OPENCODE_ROOT/agent/$SKILL_NAME.md"
fi
if [ "$HARNESS" = codex ] || [ "$HARNESS" = all ]; then
    copy_file "$SKILL_DIR/codex/prompt.md" "$CODEX_ROOT/prompts/$SKILL_NAME.md"
    copy_file "$SKILL_DIR/SKILL.md" "$CODEX_ROOT/skills/$SKILL_NAME/SKILL.md"
fi
copy_file "$SKILL_DIR/scripts/route-inventory.mjs" "$SCRIPTS_ROOT/autospec-ui-route-inventory.mjs"
if [ "$DRY_RUN" -eq 0 ]; then chmod +x "$SCRIPTS_ROOT/autospec-ui-route-inventory.mjs"; fi

[ "$UPDATE" -eq 0 ] || printf 'updated %s\n' "$SKILL_NAME"
printf 'installed %s for %s\n' "$SKILL_NAME" "$HARNESS"
