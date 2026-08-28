#!/usr/bin/env sh
# Install autospec-compose-normalize for Claude Code, OpenCode, and Codex CLI.

set -eu

SKILL_NAME=autospec-compose-normalize
SKILL_RAW_BASE="${AUTOSPEC_COMPOSE_NORMALIZE_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_COMPOSE_NORMALIZE_REF:-main}/skills/$SKILL_NAME}"
SCRIPT_PATH=${0:-}
if [ -n "$SCRIPT_PATH" ] && [ -f "$SCRIPT_PATH" ]; then
    SKILL_DIR=$(CDPATH= cd -- "$(dirname "$SCRIPT_PATH")" && pwd)
else
    SKILL_DIR=
fi

HARNESS=
USE_SYMLINK=0
DRY_RUN=0
UPDATE_MODE=0
TMP_FETCH_DIR=
SHARED_SCRIPT_FILES="claim-guard.sh lint-issue.sh worktree-guard.sh"
SHARED_LIB_SCRIPT_FILES="project-sync-issue.sh"
SKILL_SCRIPT_FILES="workflow-guard.sh"

err() { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|all] [--symlink] [--dry-run] [--update]

Installs $SKILL_NAME after verifying the top-level autospec binary supports
the deterministic Compose normalizer.
EOF
}

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        "$@"
    fi
}

cleanup() {
    if [ -n "${TMP_FETCH_DIR:-}" ] && [ -d "$TMP_FETCH_DIR" ]; then
        rm -rf "$TMP_FETCH_DIR"
    fi
}
trap cleanup EXIT HUP INT TERM

bootstrap_error() {
    err "the installed autospec binary does not provide 'runtime env normalize-compose'"
    err "upgrade the top-level suite first:"
    err "curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update"
    exit 1
}

verify_normalizer() {
    command -v autospec >/dev/null 2>&1 || bootstrap_error
    help=$(autospec runtime env --help 2>/dev/null || true)
    printf '%s\n' "$help" | \
        grep -Fq 'normalize-compose --repo PATH --check|--apply --fingerprint SHA256' \
        || bootstrap_error
}

fetch_sources() {
    command -v curl >/dev/null 2>&1 || {
        err "curl is required when install.sh is read from stdin"
        exit 1
    }
    TMP_FETCH_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t autospec)
    RAW_REPO_BASE=${SKILL_RAW_BASE%/skills/$SKILL_NAME}
    mkdir -p "$TMP_FETCH_DIR/opencode" "$TMP_FETCH_DIR/codex" \
        "$TMP_FETCH_DIR/scripts"
    for rel in SKILL.md opencode/agent.md codex/prompt.md; do
        if ! curl -fsSL "$SKILL_RAW_BASE/$rel" -o "$TMP_FETCH_DIR/$rel"; then
            err "failed to download $SKILL_RAW_BASE/$rel"
            exit 1
        fi
    done
    for rel in $SHARED_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/scripts/$rel" \
            -o "$TMP_FETCH_DIR/scripts/$rel"; then
            err "failed to download $RAW_REPO_BASE/scripts/$rel"
            exit 1
        fi
    done
    mkdir -p "$TMP_FETCH_DIR/lib-scripts"
    for rel in $SHARED_LIB_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/skills/autospec-shared/scripts/$rel" \
            -o "$TMP_FETCH_DIR/lib-scripts/$rel"; then
            err "failed to download $rel"
            exit 1
        fi
    done
    for rel in $SKILL_SCRIPT_FILES; do
        if ! curl -fsSL "$SKILL_RAW_BASE/scripts/$rel" \
            -o "$TMP_FETCH_DIR/scripts/$rel"; then
            err "failed to download $SKILL_RAW_BASE/scripts/$rel"
            exit 1
        fi
    done
    SKILL_DIR=$TMP_FETCH_DIR
}

resolve_shared_scripts_dir() {
    checkout_root=$(CDPATH= cd -- "$SKILL_DIR/../.." 2>/dev/null && pwd || true)
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/scripts" ]; then
        printf '%s\n' "$checkout_root/scripts"
    elif [ -d "$SKILL_DIR/scripts" ]; then
        printf '%s\n' "$SKILL_DIR/scripts"
    fi
}

install_one() {
    src=$1
    dest=$2
    [ -f "$src" ] || {
        err "missing source file: $src"
        return 1
    }
    run mkdir -p "$(dirname "$dest")"
    if [ "$USE_SYMLINK" -eq 1 ]; then
        run rm -f "$dest"
        run ln -s "$src" "$dest"
        info "  symlinked: $dest -> $src"
    else
        run cp "$src" "$dest"
        info "  installed: $dest"
    fi
}

install_shared_scripts() {
    src_dir=$(resolve_shared_scripts_dir)
    [ -n "$src_dir" ] || {
        err "missing shared scripts directory; cannot install runtime helpers"
        return 1
    }
    for rel in $SHARED_SCRIPT_FILES; do
        install_one "$src_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        run chmod +x "$HOME/.autospec/scripts/$rel"
    done
    checkout_root=$(CDPATH= cd -- "$SKILL_DIR/../.." 2>/dev/null && pwd || true)
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/skills/autospec-shared/scripts" ]; then
        lib_dir="$checkout_root/skills/autospec-shared/scripts"
    else
        lib_dir="$SKILL_DIR/lib-scripts"
    fi
    for rel in $SHARED_LIB_SCRIPT_FILES; do
        install_one "$lib_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        run chmod +x "$HOME/.autospec/scripts/$rel"
    done
}

install_skill_scripts() {
    for rel in $SKILL_SCRIPT_FILES; do
        install_one "$SKILL_DIR/scripts/$rel" \
            "$HOME/.autospec/scripts/autospec-compose-normalize-guard.sh"
        run chmod +x "$HOME/.autospec/scripts/autospec-compose-normalize-guard.sh"
    done
}

while [ $# -gt 0 ]; do
    case "$1" in
        --harness)
            shift
            HARNESS=${1:-}
            ;;
        --harness=*) HARNESS=${1#--harness=} ;;
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
        info "Which harness should receive $SKILL_NAME?"
        info "  1) claude  2) opencode  3) codex  4) all"
        printf 'choice [4]: '
        read -r choice || choice=
        case ${choice:-4} in
            1) HARNESS=claude ;;
            2) HARNESS=opencode ;;
            3) HARNESS=codex ;;
            4|'') HARNESS=all ;;
            claude|opencode|codex|all) HARNESS=$choice ;;
            *) err "invalid choice: $choice"; exit 2 ;;
        esac
    else
        HARNESS=all
    fi
fi
case "$HARNESS" in
    claude|opencode|codex|all) ;;
    *) err "invalid --harness: $HARNESS"; exit 2 ;;
esac

verify_normalizer

if [ -z "$SKILL_DIR" ] || [ ! -f "$SKILL_DIR/SKILL.md" ] || \
   [ ! -f "$SKILL_DIR/opencode/agent.md" ] || [ ! -f "$SKILL_DIR/codex/prompt.md" ]; then
    if [ "$USE_SYMLINK" -eq 1 ]; then
        err "--symlink requires a local checkout"
        exit 2
    fi
    fetch_sources
fi

info ""
info "Shared autospec helper scripts:"
install_shared_scripts
install_skill_scripts

CLAUDE_DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
OPENCODE_DIR=${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}
CODEX_DIR=${CODEX_HOME:-$HOME/.codex}

installed=
if [ "$HARNESS" = claude ] || [ "$HARNESS" = all ]; then
    info "\nClaude Code:"
    install_one "$SKILL_DIR/SKILL.md" "$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
    installed="${installed}  $CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md\n"
fi
if [ "$HARNESS" = opencode ] || [ "$HARNESS" = all ]; then
    info "\nOpenCode:"
    install_one "$SKILL_DIR/opencode/agent.md" "$OPENCODE_DIR/agent/$SKILL_NAME.md"
    installed="${installed}  $OPENCODE_DIR/agent/$SKILL_NAME.md\n"
fi
if [ "$HARNESS" = codex ] || [ "$HARNESS" = all ]; then
    info "\nCodex CLI:"
    install_one "$SKILL_DIR/codex/prompt.md" "$CODEX_DIR/prompts/$SKILL_NAME.md"
    install_one "$SKILL_DIR/SKILL.md" "$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"
    installed="${installed}  $CODEX_DIR/prompts/$SKILL_NAME.md\n  $CODEX_DIR/skills/$SKILL_NAME/SKILL.md\n"
fi

info ""
if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were written."
elif [ "$UPDATE_MODE" -eq 1 ]; then
    info "Updated:"
    printf '%b' "$installed"
else
    info "Installed:"
    printf '%b' "$installed"
fi
