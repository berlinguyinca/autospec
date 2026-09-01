#!/usr/bin/env sh
# install.sh — install the autospec-resume skill into one or more
# agent harnesses (Claude Code, OpenCode, Codex CLI).
#
# Usage:
#   ./install.sh                     # interactive — prompts for harness
#   ./install.sh --harness <name>    # one of: claude | opencode | codex | pi | all
#   ./install.sh --symlink           # symlink instead of copy (updates propagate)
#   ./install.sh --dry-run           # print what would be done; do nothing
#   ./install.sh --update            # idempotent re-install (overwrite existing)
#
# Can also be piped from curl:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-resume/install.sh \
#     | sh -s -- --harness all
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)
#   AUTOSPEC_RESUME_REF         (default: main)
#   AUTOSPEC_RESUME_RAW_BASE    (override raw URL base entirely)

set -eu

SKILL_NAME="autospec-resume"
SKILL_RAW_BASE="${AUTOSPEC_RESUME_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_RESUME_REF:-main}/skills/autospec-resume}"

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
# Repo-root scripts/ helpers this skill needs at runtime.
SHARED_SCRIPT_FILES="autospec-run-registry.sh heartbeat-read.sh heartbeat-write.sh autospec-stop.sh"
# Per-skill scripts/ helpers (skills/autospec-resume/scripts/).
SKILL_SCRIPT_FILES="resume-scan.sh resume-attempts.sh"

# ---------- helpers --------------------------------------------------------

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|pi|all] [--symlink] [--dry-run] [--update]

Installs the ${SKILL_NAME} skill into the chosen harness's standard
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

cleanup() {
    if [ -n "${TMP_FETCH_DIR:-}" ] && [ -d "$TMP_FETCH_DIR" ]; then
        rm -rf "$TMP_FETCH_DIR"
    fi
}
trap cleanup EXIT INT TERM

fetch_source_files() {
    if ! command -v curl >/dev/null 2>&1; then
        err "curl is required when running from stdin."
        exit 1
    fi
    TMP_FETCH_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t autospec)"
    info "Fetching ${SKILL_NAME} source files from ${SKILL_RAW_BASE} ..."
    RAW_REPO_BASE="${SKILL_RAW_BASE%/skills/$SKILL_NAME}"
    mkdir -p "$TMP_FETCH_DIR/opencode" "$TMP_FETCH_DIR/codex" "$TMP_FETCH_DIR/scripts"
    for rel in SKILL.md opencode/agent.md codex/prompt.md; do
        if ! curl -fsSL "$SKILL_RAW_BASE/$rel" -o "$TMP_FETCH_DIR/$rel"; then
            err "failed to download $SKILL_RAW_BASE/$rel"
            exit 1
        fi
    done
    for rel in $SKILL_SCRIPT_FILES; do
        if ! curl -fsSL "$SKILL_RAW_BASE/scripts/$rel" -o "$TMP_FETCH_DIR/scripts/$rel"; then
            err "failed to download $SKILL_RAW_BASE/scripts/$rel"
            exit 1
        fi
    done
    mkdir -p "$TMP_FETCH_DIR/repo-scripts"
    for rel in $SHARED_SCRIPT_FILES; do
        # heartbeat-*.sh live under skills/autospec-run/scripts/.
        case "$rel" in
            heartbeat-read.sh|heartbeat-write.sh)
                src_url="$RAW_REPO_BASE/skills/autospec-run/scripts/$rel" ;;
            *)
                src_url="$RAW_REPO_BASE/scripts/$rel" ;;
        esac
        if ! curl -fsSL "$src_url" -o "$TMP_FETCH_DIR/repo-scripts/$rel"; then
            err "failed to download $src_url"
            exit 1
        fi
    done
    SKILL_DIR="$TMP_FETCH_DIR"
}

# resolve_repo_scripts_dir — locate the repo-root scripts/ in a full checkout,
# or the fetched repo-scripts/ dir when running from stdin.
resolve_repo_root() {
    cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true
}

install_runtime_scripts() {
    checkout_root="$(resolve_repo_root)"
    # Per-skill scripts.
    if [ -n "$checkout_root" ] && [ -d "$SKILL_DIR/scripts" ]; then
        per_skill_dir="$SKILL_DIR/scripts"
    else
        per_skill_dir="$SKILL_DIR/scripts"
    fi
    for rel in $SKILL_SCRIPT_FILES; do
        install_one "$per_skill_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        run "chmod +x \"$HOME/.autospec/scripts/$rel\""
    done
    # Shared repo-root scripts (and autospec-run scripts).
    for rel in $SHARED_SCRIPT_FILES; do
        src=""
        if [ -n "$checkout_root" ]; then
            case "$rel" in
                heartbeat-read.sh|heartbeat-write.sh)
                    src="$checkout_root/skills/autospec-run/scripts/$rel" ;;
                *)
                    src="$checkout_root/scripts/$rel" ;;
            esac
        fi
        if [ -z "$src" ] || [ ! -f "$src" ]; then
            src="$SKILL_DIR/repo-scripts/$rel"
        fi
        install_one "$src" "$HOME/.autospec/scripts/$rel" || return 1
        run "chmod +x \"$HOME/.autospec/scripts/$rel\""
    done
}

install_one() {
    src="$1"
    dest="$2"
    dest_dir="$(dirname "$dest")"

    if [ ! -f "$src" ]; then
        err "missing source file: $src"
        return 1
    fi

    run "mkdir -p \"$dest_dir\""

    if [ "$USE_SYMLINK" -eq 1 ]; then
        run "rm -f \"$dest\""
        run "ln -s \"$src\" \"$dest\""
        info "  symlinked: $dest -> $src"
    else
        run "cp \"$src\" \"$dest\""
        info "  installed: $dest"
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
        --symlink)
            USE_SYMLINK=1
            ;;
        --dry-run)
            DRY_RUN=1
            ;;
        --update)
            UPDATE_MODE=1
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

# ---------- harness prompt -------------------------------------------------

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

# ---------- ensure source files are available ------------------------------

need_fetch=0
if [ -z "$SKILL_DIR" ]; then
    need_fetch=1
elif [ ! -f "$SKILL_DIR/SKILL.md" ] || [ ! -f "$SKILL_DIR/opencode/agent.md" ] || [ ! -f "$SKILL_DIR/codex/prompt.md" ]; then
    need_fetch=1
fi

if [ "$need_fetch" -eq 1 ]; then
    if [ "$USE_SYMLINK" -eq 1 ]; then
        err "--symlink requires running from a checkout (no local files to symlink to)."
        exit 2
    fi
    fetch_source_files
fi

# ---------- install runtime helper scripts ---------------------------------

info ""
info "autospec-resume runtime helper scripts:"
install_runtime_scripts

# ---------- per-harness paths ---------------------------------------------

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
PI_DIR="${PI_SKILLS_DIR:-$HOME/.agents/skills}"

CLAUDE_DEST="$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
OPENCODE_DEST="$OPENCODE_DIR/agent/$SKILL_NAME.md"
CODEX_DEST="$CODEX_DIR/prompts/$SKILL_NAME.md"
PI_DEST="$PI_DIR/$SKILL_NAME/SKILL.md"

CLAUDE_SRC="$SKILL_DIR/SKILL.md"
OPENCODE_SRC="$SKILL_DIR/opencode/agent.md"
CODEX_SRC="$SKILL_DIR/codex/prompt.md"
PI_SRC="$SKILL_DIR/SKILL.md"

# ---------- install --------------------------------------------------------

installed_paths=""

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Claude Code:"
    install_one "$CLAUDE_SRC" "$CLAUDE_DEST"
    installed_paths="${installed_paths}  Claude Code: ${CLAUDE_DEST}\n"
fi

if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "OpenCode:"
    install_one "$OPENCODE_SRC" "$OPENCODE_DEST"
    installed_paths="${installed_paths}  OpenCode:    ${OPENCODE_DEST}\n"
fi

if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Codex CLI:"
    install_one "$CODEX_SRC" "$CODEX_DEST"
    install_one "$CLAUDE_SRC" "$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"
    installed_paths="${installed_paths}  Codex CLI:   ${CODEX_DEST}\n"
    installed_paths="${installed_paths}  Codex skill: ${CODEX_DIR}/skills/${SKILL_NAME}/SKILL.md\n"
fi

if [ "$HARNESS" = "pi" ] || [ "$HARNESS" = "all" ]; then
    info ""
    info "Pi:"
    install_one "$PI_SRC" "$PI_DEST"
    installed_paths="${installed_paths}  Pi:        ${PI_DEST}\n"
fi

# ---------- final summary --------------------------------------------------

info ""
if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were written."
else
    if [ "$UPDATE_MODE" -eq 1 ]; then
        info "Updated (idempotent overwrite):"
    else
        info "Installed:"
    fi
    printf '%b' "$installed_paths"
    info ""
    info "Invoke with:"
    info "  Claude Code:  /${SKILL_NAME}"
    info "  OpenCode:     @${SKILL_NAME} --dry-run"
    info "  Codex CLI:    /${SKILL_NAME} --resume-partial"
fi

exit 0
