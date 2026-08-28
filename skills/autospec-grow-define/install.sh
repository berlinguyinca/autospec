#!/usr/bin/env sh
# install.sh — install the autospec skill into one or more
# agent harnesses (Claude Code, OpenCode, Codex CLI).
#
# Usage:
#   ./install.sh                     # interactive — prompts for harness
#   ./install.sh --harness <name>    # one of: claude | opencode | codex | all
#   ./install.sh --symlink           # symlink instead of copy (updates propagate)
#   ./install.sh --dry-run           # print what would be done; do nothing
#
# Can also be piped from curl:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-define/install.sh \
#     | sh -s -- --harness all
# When piped, the script auto-downloads the skill files from the same branch.
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)
#   AUTOSPEC_GROW_DEFINE_REF         (default: main) — git ref to fetch from when piped
#   AUTOSPEC_GROW_DEFINE_RAW_BASE    (override the raw URL base entirely)
#
# Idempotent: re-running upgrades the install. Exits non-zero on hard failure;
# exits zero with warnings on missing optional deps.

set -eu

SKILL_NAME="autospec-grow-define"
SKILL_RAW_BASE="${AUTOSPEC_GROW_DEFINE_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_GROW_DEFINE_REF:-main}/skills/autospec-grow-define}"

# Resolve the directory containing this script. When piped through stdin (e.g.
# `curl ... | sh`), $0 will not be a real file — detect that and fall back to
# downloading the source files from the raw GitHub URL.
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
SHARED_SCRIPT_FILES="autospec-usage-limit.sh autospec-stop.sh autospec-watchdog.sh autospec-watchdog.ps1 lint-implementation.sh lint-issue.sh listener-match.sh sizing-check.sh ci-wait.sh ci-wait-poll.sh ci-wait-cleanup.sh gen-implementer-prompt.sh gen-reviewer-prompt.sh"
# Shared runtime helpers this skill calls via ${AUTOSPEC_SCRIPTS_DIR}/<name> and
# that live under skills/autospec-shared/scripts/ (not repo-root scripts/). They
# MUST ship into ~/.autospec/scripts so a standalone install of
# autospec-grow-define can run the skill in a target repo without this checkout
# present.
SHARED_LIB_SCRIPT_FILES="inject-relevant-memory.sh validate-growth-config.sh validate-growth-candidate.sh growth-measure.sh growth-source-weights.sh growth-candidate-dedup.sh growth-candidate-verify.sh growth-candidate-rank.sh growth-ledger.sh grow-define-pipeline.sh grow-define-file-issues.sh project-sync-issue.sh"

# ---------- helpers --------------------------------------------------------

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|all] [--symlink] [--dry-run] [--update]

Installs the ${SKILL_NAME} skill into the chosen harness's standard
skill/agent/prompt directory.

Examples:
  $0                          # interactive
  $0 --harness all            # install everywhere
  $0 --harness claude         # Claude Code only
  $0 --harness opencode       # OpenCode only
  $0 --harness codex          # Codex CLI only
  $0 --harness all --symlink  # symlink instead of copy
  $0 --dry-run --harness all  # print plan, change nothing
  $0 --harness claude --update  # idempotent re-install (overwrite existing)
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
    # Download the files we need into a temp dir and set SKILL_DIR to it.
    if ! command -v curl >/dev/null 2>&1; then
        err "curl is required when running from stdin (e.g. piped via 'curl | sh')."
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
    for rel in $SHARED_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/scripts/$rel" -o "$TMP_FETCH_DIR/scripts/$rel"; then
            err "failed to download $RAW_REPO_BASE/scripts/$rel"
            exit 1
        fi
    done
    mkdir -p "$TMP_FETCH_DIR/lib-scripts"
    for rel in $SHARED_LIB_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/skills/autospec-shared/scripts/$rel" \
            -o "$TMP_FETCH_DIR/lib-scripts/$rel"; then
            err "failed to download $RAW_REPO_BASE/skills/autospec-shared/scripts/$rel"
            exit 1
        fi
    done
    SKILL_DIR="$TMP_FETCH_DIR"
}

resolve_shared_scripts_dir() {
    checkout_root="$(cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true)"
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/scripts" ]; then
        printf '%s\n' "$checkout_root/scripts"
    elif [ -d "$SKILL_DIR/scripts" ]; then
        printf '%s\n' "$SKILL_DIR/scripts"
    else
        printf ''
    fi
}

install_shared_scripts() {
    src_dir="$(resolve_shared_scripts_dir)"
    if [ -z "$src_dir" ]; then
        err "missing shared scripts directory; cannot install autospec helper scripts"
        return 1
    fi
    for rel in $SHARED_SCRIPT_FILES; do
        install_one "$src_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        case "$rel" in
            *.sh) run "chmod +x \"$HOME/.autospec/scripts/$rel\"" ;;
        esac
    done
}

# resolve_shared_lib_scripts_dir — locate skills/autospec-shared/scripts/ in a
# full checkout, or the fetched lib-scripts/ dir when running from stdin.
resolve_shared_lib_scripts_dir() {
    checkout_root="$(cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true)"
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/skills/autospec-shared/scripts" ]; then
        printf '%s\n' "$checkout_root/skills/autospec-shared/scripts"
    elif [ -d "$SKILL_DIR/lib-scripts" ]; then
        printf '%s\n' "$SKILL_DIR/lib-scripts"
    else
        printf ''
    fi
}

# install_shared_lib_scripts — ship the shared-lib runtime helpers this skill
# calls at runtime into ~/.autospec/scripts so the skill works standalone.
install_shared_lib_scripts() {
    lib_dir="$(resolve_shared_lib_scripts_dir)"
    if [ -z "$lib_dir" ]; then
        err "missing shared-lib scripts directory; cannot install autospec-grow-define helpers"
        return 1
    fi
    for rel in $SHARED_LIB_SCRIPT_FILES; do
        install_one "$lib_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        case "$rel" in
            *.sh) run "chmod +x \"$HOME/.autospec/scripts/$rel\"" ;;
        esac
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

# ---------- ensure source files are available ------------------------------

# If we couldn't resolve a local SKILL_DIR (piped from curl) OR the local dir
# doesn't actually contain the skill files, fall back to remote fetch.
need_fetch=0
if [ -z "$SKILL_DIR" ]; then
    need_fetch=1
elif [ ! -f "$SKILL_DIR/SKILL.md" ] || [ ! -f "$SKILL_DIR/opencode/agent.md" ] || [ ! -f "$SKILL_DIR/codex/prompt.md" ]; then
    need_fetch=1
fi

if [ "$need_fetch" -eq 1 ]; then
    if [ "$USE_SYMLINK" -eq 1 ]; then
        err "--symlink requires running from a checkout of the codex-skills repo (no local files to symlink to)."
        exit 2
    fi
    fetch_source_files
fi

# ---------- dependency checks ---------------------------------------------

dep_warnings=0

check_dep() {
    name="$1"
    if ! command -v "$name" >/dev/null 2>&1; then
        warn "$name not found on PATH (required by the skill at runtime)"
        dep_warnings=$((dep_warnings + 1))
    fi
}

check_dep git
check_dep gh
check_dep jq

if command -v gh >/dev/null 2>&1; then
    if ! gh auth status >/dev/null 2>&1; then
        warn "gh is installed but not authenticated. Run: gh auth login"
        dep_warnings=$((dep_warnings + 1))
    fi
fi

if [ "$dep_warnings" -gt 0 ]; then
    warn "${dep_warnings} dependency warning(s) above. The skill files will still install."
fi

# ---------- install shared helper scripts ---------------------------------

info ""
info "Shared autospec helper scripts:"
install_shared_scripts
install_shared_lib_scripts

# ---------- per-harness paths ---------------------------------------------

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"

CLAUDE_DEST="$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
OPENCODE_DEST="$OPENCODE_DIR/agent/$SKILL_NAME.md"
CODEX_DEST="$CODEX_DIR/prompts/$SKILL_NAME.md"

CLAUDE_SRC="$SKILL_DIR/SKILL.md"
OPENCODE_SRC="$SKILL_DIR/opencode/agent.md"
CODEX_SRC="$SKILL_DIR/codex/prompt.md"

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
    info "  Claude Code:  /${SKILL_NAME} <run this cycle / update>"
    info "  OpenCode:     @${SKILL_NAME} <run this cycle / update>"
    info "  Codex CLI:    /${SKILL_NAME} <run this cycle / update>"
    info ""
    info "Opt in per repo by creating .autospec/growth.yml before invoking."
fi

exit 0
