#!/usr/bin/env sh
# install.sh - install the autospec-project skill into one or more harnesses.

set -eu

SKILL_NAME="autospec-project"
SKILL_RAW_BASE="${AUTOSPEC_PROJECT_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_PROJECT_REF:-main}/skills/autospec-project}"
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
SHARED_SCRIPT_FILES="
project-board-resolve.sh
project-board-normalize.sh
project-board-deps.sh
project-board-writeback.sh
autonomous-promote-open-issues.sh
project-ship.sh
"
# project-ship.sh (the `ship` chain) sources fleet-lib.sh and shells out to
# fleet-run.sh at runtime — both must land alongside it or `ship` hard-
# crashes on a clean install. Mirrors autospec-fleet/install.sh's own
# FLEET_SCRIPT_FILES set (fleet-config-lint.sh is fleet-run.sh's own
# same-directory dependency, so it rides along too).
FLEET_SCRIPT_FILES="fleet-run.sh fleet-lib.sh fleet-config-lint.sh"

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|pi|all] [--symlink] [--dry-run] [--update]

Installs the ${SKILL_NAME} skill into the chosen harness.
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
    mkdir -p "$TMP_FETCH_DIR/opencode" "$TMP_FETCH_DIR/codex" "$TMP_FETCH_DIR/scripts"
    for rel in SKILL.md opencode/agent.md codex/prompt.md; do
        curl -fsSL "$SKILL_RAW_BASE/$rel" -o "$TMP_FETCH_DIR/$rel" || { err "failed to download $rel"; exit 1; }
    done
    for rel in $SHARED_SCRIPT_FILES; do
        curl -fsSL "$RAW_REPO_BASE/scripts/$rel" -o "$TMP_FETCH_DIR/scripts/$rel" || { err "failed to download $rel"; exit 1; }
    done
    for rel in $FLEET_SCRIPT_FILES; do
        curl -fsSL "$RAW_REPO_BASE/skills/autospec-fleet/scripts/$rel" -o "$TMP_FETCH_DIR/scripts/$rel" || { err "failed to download $rel"; exit 1; }
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

resolve_fleet_scripts_dir() {
    checkout_root="$(cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true)"
    if [ -n "$checkout_root" ] && [ -d "$checkout_root/skills/autospec-fleet/scripts" ]; then
        printf '%s\n' "$checkout_root/skills/autospec-fleet/scripts"
    elif [ -d "$SKILL_DIR/scripts" ]; then
        # Fetched (curl) layout: fleet scripts land alongside the shared
        # ones in the same flat scripts/ dir — see fetch_source_files.
        printf '%s\n' "$SKILL_DIR/scripts"
    else
        printf ''
    fi
}

managed_project_command_available() {
    onboard_help="$("$1" project onboard --help 2>/dev/null)" || return 1
    sync_help="$("$1" project sync --help 2>/dev/null)" || return 1
    printf '%s\n' "$onboard_help" | grep -F 'autospec project onboard' >/dev/null || return 1
    printf '%s\n' "$onboard_help" | grep -F -- '--spawned-from' >/dev/null || return 1
    printf '%s\n' "$sync_help" | grep -F 'autospec project sync' >/dev/null || return 1
}

ensure_managed_project_runtime() {
    autospec_candidate="${AUTOSPEC_BIN:-}"
    if [ -z "$autospec_candidate" ]; then
        autospec_candidate="$(command -v autospec 2>/dev/null || true)"
    fi
    if [ -n "$autospec_candidate" ] && managed_project_command_available "$autospec_candidate"; then
        return 0
    fi

    checkout_root="$(cd "$SKILL_DIR/../.." 2>/dev/null && pwd || true)"
    runtime_installer="${AUTOSPEC_PROJECT_RUNTIME_INSTALLER:-}"
    if [ -z "$runtime_installer" ] && [ -n "$checkout_root" ]; then
        runtime_installer="$checkout_root/scripts/autospec-runtime-install.sh"
    fi
    if [ "$DRY_RUN" -eq 1 ]; then
        [ -n "$runtime_installer" ] && [ -f "$runtime_installer" ] || {
            err "autospec project modes are unavailable and no runtime installer was found"
            return 1
        }
        info "  [dry-run] $runtime_installer --repo-dir $checkout_root"
        return 0
    fi
    [ -n "$runtime_installer" ] && [ -x "$runtime_installer" ] || {
        err "autospec project modes are unavailable and no executable runtime installer was found"
        return 1
    }
    runtime_path="$("$runtime_installer" --repo-dir "$checkout_root")" || {
        err "autospec project modes are unavailable because runtime installation failed"
        return 1
    }
    runtime_path="$(printf '%s\n' "$runtime_path" | tail -n 1)"
    if [ -z "$runtime_path" ] || ! managed_project_command_available "$runtime_path"; then
        err "autospec project modes are unavailable after runtime installation"
        return 1
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

# project-ship.sh sources fleet-lib.sh and shells out to fleet-run.sh at its
# own directory (falling back to the flattened $HOME/.autospec/scripts/
# layout every autospec script targets) — installed alongside the shared
# scripts above, same destination directory, so both resolution paths land
# on the same files.
install_fleet_scripts() {
    src_dir="$(resolve_fleet_scripts_dir)"
    [ -n "$src_dir" ] || { err "missing fleet scripts directory"; return 1; }
    for rel in $FLEET_SCRIPT_FILES; do
        install_one "$src_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        run "chmod +x \"$HOME/.autospec/scripts/$rel\""
    done
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

[ -n "$HARNESS" ] || HARNESS=all
case "$HARNESS" in claude|opencode|codex|all) ;; *) err "invalid --harness: $HARNESS"; exit 2 ;; esac

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

for dep in autospec git gh jq; do
    command -v "$dep" >/dev/null 2>&1 || warn "$dep not found on PATH"
done

ensure_managed_project_runtime

info ""
info "Shared autospec helper scripts:"
install_shared_scripts

info ""
info "autospec-fleet scripts (required by ship):"
install_fleet_scripts

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
PI_DIR="${PI_SKILLS_DIR:-$HOME/.agents/skills}"

if [ "$HARNESS" = "claude" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Claude Code:"
    install_one "$SKILL_DIR/SKILL.md" "$CLAUDE_DIR/skills/$SKILL_NAME/SKILL.md"
fi
if [ "$HARNESS" = "opencode" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "OpenCode:"
    install_one "$SKILL_DIR/opencode/agent.md" "$OPENCODE_DIR/agent/$SKILL_NAME.md"
fi
if [ "$HARNESS" = "codex" ] || [ "$HARNESS" = "all" ]; then
    info ""; info "Codex CLI:"
    install_one "$SKILL_DIR/codex/prompt.md" "$CODEX_DIR/prompts/$SKILL_NAME.md"
    install_one "$SKILL_DIR/SKILL.md" "$CODEX_DIR/skills/$SKILL_NAME/SKILL.md"
fi

info ""
if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. No files were written."
elif [ "$UPDATE_MODE" -eq 1 ]; then
    info "Updated $SKILL_NAME."
else
    info "Installed $SKILL_NAME."
fi
