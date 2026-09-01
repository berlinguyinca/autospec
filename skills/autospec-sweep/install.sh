#!/usr/bin/env sh
# install.sh — install the autospec-sweep skill into one or more harnesses.

set -eu

SKILL_NAME="autospec-sweep"
SKILL_RAW_BASE="${AUTOSPEC_SWEEP_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_SWEEP_REF:-main}/skills/autospec-sweep}"
SHARED_SCRIPT_FILES="autospec-usage-limit.sh autospec-stop.sh autospec-watchdog.sh autospec-watchdog.ps1 lint-implementation.sh lint-issue.sh listener-match.sh sizing-check.sh ci-wait.sh ci-wait-poll.sh ci-wait-cleanup.sh gen-implementer-prompt.sh gen-reviewer-prompt.sh"

SCRIPT_PATH="${0:-}"
if [ -n "$SCRIPT_PATH" ] && [ -f "$SCRIPT_PATH" ]; then
    SKILL_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
else
    SKILL_DIR=""
fi

HARNESS=""
USE_SYMLINK=0
DRY_RUN=0
UPDATE_MODE=0
TMP_FETCH_DIR=""

err() { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|pi|all] [--symlink] [--dry-run] [--update]

Installs the ${SKILL_NAME} skill and its first-run wizard.
EOF
}

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        eval "$@"
    fi
}

cleanup() {
    if [ -n "${TMP_FETCH_DIR:-}" ] && [ -d "$TMP_FETCH_DIR" ]; then
        rm -rf "$TMP_FETCH_DIR"
    fi
}
trap cleanup EXIT INT TERM

fetch_source_files() {
    command -v curl >/dev/null 2>&1 || { err "curl is required when running from stdin"; exit 1; }
    TMP_FETCH_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t autospec)"
    RAW_REPO_BASE="${SKILL_RAW_BASE%/skills/$SKILL_NAME}"
    mkdir -p "$TMP_FETCH_DIR/opencode" "$TMP_FETCH_DIR/codex" "$TMP_FETCH_DIR/scripts" "$TMP_FETCH_DIR/shared-scripts"
    for rel in SKILL.md opencode/agent.md codex/prompt.md scripts/wizard.sh scripts/run.sh scripts/review.sh; do
        if ! curl -fsSL "$SKILL_RAW_BASE/$rel" -o "$TMP_FETCH_DIR/$rel"; then
            err "failed to download $SKILL_RAW_BASE/$rel"
            exit 1
        fi
    done
    for rel in $SHARED_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/scripts/$rel" -o "$TMP_FETCH_DIR/shared-scripts/$rel"; then
            err "failed to download $RAW_REPO_BASE/scripts/$rel"
            exit 1
        fi
    done
    SKILL_DIR="$TMP_FETCH_DIR"
}

resolve_shared_scripts_dir() {
    checkout_root="$(cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true)"
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/scripts" ]; then
        printf '%s\n' "$checkout_root/scripts"
    elif [ -d "$SKILL_DIR/shared-scripts" ]; then
        printf '%s\n' "$SKILL_DIR/shared-scripts"
    else
        printf ''
    fi
}

install_one() {
    src="$1"
    dest="$2"
    [ -f "$src" ] || { err "missing source file: $src"; return 1; }
    run "mkdir -p \"$(dirname "$dest")\""
    if [ "$USE_SYMLINK" -eq 1 ]; then
        run "rm -f \"$dest\""
        run "ln -s \"$src\" \"$dest\""
        info "  symlinked: $dest -> $src"
    else
        run "cp \"$src\" \"$dest\""
        info "  installed: $dest"
    fi
}

install_shared_scripts() {
    src_dir="$(resolve_shared_scripts_dir)"
    [ -n "$src_dir" ] || { err "missing shared scripts directory"; return 1; }
    for rel in $SHARED_SCRIPT_FILES; do
        install_one "$src_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        case "$rel" in *.sh) run "chmod +x \"$HOME/.autospec/scripts/$rel\"" ;; esac
    done
}

install_sweep_scripts() {
    install_one "$SKILL_DIR/scripts/wizard.sh" "$HOME/.autospec/scripts/autospec-sweep-wizard.sh"
    run "chmod +x \"$HOME/.autospec/scripts/autospec-sweep-wizard.sh\""
    install_one "$SKILL_DIR/scripts/run.sh" "$HOME/.autospec/scripts/autospec-sweep-run.sh"
    run "chmod +x \"$HOME/.autospec/scripts/autospec-sweep-run.sh\""
    install_one "$SKILL_DIR/scripts/review.sh" "$HOME/.autospec/scripts/autospec-sweep-review.sh"
    run "chmod +x \"$HOME/.autospec/scripts/autospec-sweep-review.sh\""
}

while [ $# -gt 0 ]; do
    case "$1" in
        --harness) shift; HARNESS="${1:-}" ;;
        --harness=*) HARNESS="${1#--harness=}" ;;
        --symlink) USE_SYMLINK=1 ;;
        --dry-run) DRY_RUN=1 ;;
        --update) UPDATE_MODE=1 ;;
        -h|--help) usage; exit 0 ;;
        *) err "unknown arg: $1"; usage; exit 2 ;;
    esac
    shift
done

if [ -z "$HARNESS" ]; then
    if [ -t 0 ] && [ -t 1 ]; then
        info "Which harness should we install for?"
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

need_fetch=0
if [ -z "$SKILL_DIR" ]; then
    need_fetch=1
elif [ ! -f "$SKILL_DIR/SKILL.md" ] || [ ! -f "$SKILL_DIR/opencode/agent.md" ] || [ ! -f "$SKILL_DIR/codex/prompt.md" ]; then
    need_fetch=1
fi

if [ "$need_fetch" -eq 1 ]; then
    [ "$USE_SYMLINK" -eq 0 ] || { err "--symlink requires a local checkout"; exit 2; }
    fetch_source_files
fi

for dep in git gh jq yq; do
    command -v "$dep" >/dev/null 2>&1 || warn "$dep not found on PATH"
done

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
PI_DIR="${PI_SKILLS_DIR:-$HOME/.agents/skills}"

CLAUDE_DEST="$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
OPENCODE_DEST="$OPENCODE_DIR/agent/$SKILL_NAME.md"
CODEX_DEST="$CODEX_DIR/prompts/$SKILL_NAME.md"
PI_DEST="$PI_DIR/$SKILL_NAME/SKILL.md"
CODEX_SKILLS_DEST="$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"

install_shared_scripts
install_sweep_scripts

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Claude Code:"
    install_one "$SKILL_DIR/SKILL.md" "$CLAUDE_DEST"
fi

if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "OpenCode:"
    install_one "$SKILL_DIR/opencode/agent.md" "$OPENCODE_DEST"
fi

if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Codex CLI:"
    install_one "$SKILL_DIR/codex/prompt.md" "$CODEX_DEST"
    install_one "$SKILL_DIR/SKILL.md" "$CODEX_SKILLS_DEST"
fi

[ "$UPDATE_MODE" -eq 0 ] || info "Update mode complete."
info "Done. Run /autospec-sweep init to create .autospec/autospec.yml."
