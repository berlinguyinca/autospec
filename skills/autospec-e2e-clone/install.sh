#!/usr/bin/env sh
# install.sh — install the autospec-e2e-clone skill into one or more
# agent harnesses (Claude Code, OpenCode, Codex CLI).
#
# Usage:
#   ./install.sh                     # interactive — prompts for harness
#   ./install.sh --harness <name>    # one of: claude | opencode | codex | pi | all
#   ./install.sh --update            # update an existing install
#   ./install.sh --symlink           # symlink instead of copy
#   ./install.sh --dry-run           # print what would be done; do nothing
#
# Can also be piped from curl:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-e2e-clone/install.sh \
#     | sh -s -- --harness all
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)
#   AUTOSPEC_E2E_CLONE_REF         (default: main)
#   AUTOSPEC_E2E_CLONE_RAW_BASE    (override raw URL base)

set -eu

SKILL_NAME="autospec-e2e-clone"
SKILL_RAW_BASE="${AUTOSPEC_E2E_CLONE_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_E2E_CLONE_REF:-main}/skills/autospec-e2e-clone}"

SCRIPT_PATH="${0:-}"
if [ -n "$SCRIPT_PATH" ] && [ -f "$SCRIPT_PATH" ]; then
    SKILL_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
else
    SKILL_DIR=""
fi

HARNESS=""
SYMLINK=0
DRY_RUN=0
UPDATE=0

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|pi|all] [--update] [--symlink] [--dry-run]

Installs the ${SKILL_NAME} skill into the chosen harness.
EOF
}

run_cmd() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        eval "$@"
    fi
}

fetch_file() {
    src="$1"
    dst="$2"
    if [ -n "$SKILL_DIR" ] && [ -f "$SKILL_DIR/$src" ]; then
        run_cmd "cp \"$SKILL_DIR/$src\" \"$dst\""
    else
        run_cmd "curl -fsSL \"$SKILL_RAW_BASE/$src\" -o \"$dst\""
    fi
}

install_one() {
    target="$1"
    src_file="${2:-SKILL.md}"
    target_dir="$(dirname "$target")"
    run_cmd "mkdir -p \"$target_dir\""
    fetch_file "$src_file" "$target"
    info "  installed: $target"
}

# ---------- arg parse -------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --harness)
            shift; HARNESS="${1:-}" ;;
        --harness=*)
            HARNESS="${1#--harness=}" ;;
        --update)
            UPDATE=1 ;;
        --symlink)
            SYMLINK=1 ;;
        --dry-run)
            DRY_RUN=1 ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            err "unknown arg: $1"; usage; exit 2 ;;
    esac
    shift
done

if [ -z "$HARNESS" ]; then
    if [ -t 0 ] && [ -t 1 ]; then
        info "Which harness should we install into?"
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
            claude|opencode|codex|pi|all) HARNESS="$choice" ;;
            *) err "invalid choice: $choice"; exit 2 ;;
        esac
    else
        HARNESS=all
    fi
fi

case "$HARNESS" in
    claude|opencode|codex|pi|all) ;;
    *) err "invalid --harness: $HARNESS"; usage; exit 2 ;;
esac

# ---------- per-harness paths -----------------------------------------------

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
PI_DIR="${PI_SKILLS_DIR:-$HOME/.agents/skills}"

CLAUDE_DEST="$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
OPENCODE_DEST="$OPENCODE_DIR/agent/$SKILL_NAME.md"
CODEX_DEST="$CODEX_DIR/prompts/$SKILL_NAME.md"
PI_DEST="$PI_DIR/$SKILL_NAME/SKILL.md"
CODEX_SKILLS_DEST="$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"

# ---------- install ---------------------------------------------------------

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Claude Code:"
    install_one "$CLAUDE_DEST" "SKILL.md"
fi

if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "OpenCode:"
    install_one "$OPENCODE_DEST" "opencode/agent.md"
fi

if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Codex CLI:"
    install_one "$CODEX_DEST" "codex/prompt.md"
    install_one "$CODEX_SKILLS_DEST" "SKILL.md"
fi

if [ "$HARNESS" = "pi" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Pi:"
    install_one "$PI_DEST"
fi

info ""
if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were written."
else
    info "Done. Run /autospec-e2e-clone to provision a clone environment."
fi

exit 0
