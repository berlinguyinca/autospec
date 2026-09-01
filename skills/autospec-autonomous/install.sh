#!/usr/bin/env sh
# install.sh — install the autospec skill into one or more
# agent harnesses (Claude Code, OpenCode, Codex CLI).
#
# Usage:
#   ./install.sh                     # interactive — prompts for harness
#   ./install.sh --harness <name>    # one of: claude | opencode | codex | pi | all
#   ./install.sh --symlink           # symlink instead of copy (updates propagate)
#   ./install.sh --dry-run           # print what would be done; do nothing
#
# Can also be piped from curl:
#   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/install.sh \
#     | sh -s -- --harness all
# When piped, the script auto-downloads the skill files from the same branch.
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)
#   AUTOSPEC_AUTONOMOUS_REF         (default: main) — git ref to fetch from when piped
#   AUTOSPEC_AUTONOMOUS_RAW_BASE    (override the raw URL base entirely)
#
# Idempotent: re-running upgrades the install. Exits non-zero on hard failure;
# exits zero with warnings on missing optional deps.

set -eu

SKILL_NAME="autospec-autonomous"
SKILL_RAW_BASE="${AUTOSPEC_AUTONOMOUS_RAW_BASE:-https://raw.githubusercontent.com/berlinguyinca/autospec/${AUTOSPEC_AUTONOMOUS_REF:-main}/skills/autospec-autonomous}"

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
AUTONOMOUS_SCRIPT_FILES="autospec-autonomous.sh autospec-autonomous-launcher.sh autospec-autonomous-run-drain.sh autonomous-runtime-refresh.sh autospec-runtime-install.sh autonomous-control-channel.sh autonomous-guardrails.sh autonomous-persona-sources.sh autonomous-persona-synth.sh autonomous-persona-mine.sh autonomous-priority-match.sh autonomous-premerge-gate.sh autonomous-resilience.sh autonomous-spend-ledger.sh autonomous-usage-governor.sh autonomous-waterfall.sh autospec-autonomy-gate.sh usage-observe.sh project-board-resolve.sh project-board-normalize.sh project-board-deps.sh project-board-writeback.sh project-board-control-mirror.sh autonomous-promote-open-issues.sh list-groomable.sh classify-model-fit.sh promote-eligibility.sh groom-fill.sh"
LIB_FILES="autospec-loop.sh autospec-harness-detect.sh"
# Scripts that live under skills/autospec-shared/scripts/ rather than the
# repo-root scripts/ dir, but are still required at runtime: the Tier 1.5
# grooming orchestrator (autonomous-promote-open-issues.sh) reads its policy
# through grooming-config.sh at that path.
SHARED_LIB_SCRIPT_FILES="grooming-config.sh project-sync-issue.sh"

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--harness claude|opencode|codex|pi|all] [--symlink] [--dry-run] [--update]

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
    if ! command -v curl >/dev/null 2>&1; then
        err "curl is required when running from stdin (e.g. piped via 'curl | sh')."
        exit 1
    fi
    TMP_FETCH_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t autospec)"
    info "Fetching ${SKILL_NAME} source files from ${SKILL_RAW_BASE} ..."
    RAW_REPO_BASE="${SKILL_RAW_BASE%/skills/$SKILL_NAME}"
    mkdir -p "$TMP_FETCH_DIR/opencode" "$TMP_FETCH_DIR/codex" "$TMP_FETCH_DIR/scripts" "$TMP_FETCH_DIR/scripts/lib"
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
    for rel in $AUTONOMOUS_SCRIPT_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/scripts/$rel" -o "$TMP_FETCH_DIR/scripts/$rel"; then
            err "failed to download $RAW_REPO_BASE/scripts/$rel"
            exit 1
        fi
    done
    for rel in $LIB_FILES; do
        if ! curl -fsSL "$RAW_REPO_BASE/scripts/lib/$rel" -o "$TMP_FETCH_DIR/scripts/lib/$rel"; then
            err "failed to download $RAW_REPO_BASE/scripts/lib/$rel"
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

install_autonomous_scripts() {
    src_dir="$(resolve_shared_scripts_dir)"
    if [ -z "$src_dir" ]; then
        err "missing shared scripts directory; cannot install autonomous helper scripts"
        return 1
    fi
    for rel in $AUTONOMOUS_SCRIPT_FILES; do
        install_one "$src_dir/$rel" "$HOME/.autospec/scripts/$rel" || return 1
        run "chmod +x \"$HOME/.autospec/scripts/$rel\""
    done
    # Install shared-lib scripts under $HOME/.autospec/skills/autospec-shared/scripts/,
    # mirroring the repo-relative layout that autonomous-promote-open-issues.sh
    # resolves grooming-config.sh through (SCRIPT_DIR/../skills/autospec-shared/scripts) —
    # a flat $HOME/.autospec/scripts/ install would leave it unfindable at runtime.
    lib_dir="$(resolve_shared_lib_scripts_dir)"
    if [ -z "$lib_dir" ]; then
        err "missing shared-lib scripts directory; cannot install grooming dependencies"
        return 1
    fi
    for rel in $SHARED_LIB_SCRIPT_FILES; do
        install_one "$lib_dir/$rel" "$HOME/.autospec/skills/autospec-shared/scripts/$rel" || return 1
        run "chmod +x \"$HOME/.autospec/skills/autospec-shared/scripts/$rel\""
    done
}

install_lib_files() {
    src_dir="$(resolve_shared_scripts_dir)"
    if [ -z "$src_dir" ]; then
        err "missing shared scripts directory; cannot install runtime lib files"
        return 1
    fi
    for rel in $LIB_FILES; do
        install_one "$src_dir/lib/$rel" "$HOME/.autospec/scripts/lib/$rel" || return 1
        case "$rel" in
            *.sh) run "chmod +x \"$HOME/.autospec/scripts/lib/$rel\"" ;;
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

ensure_autospec_bin_path() {
    autospec_bin_dir="$HOME/.autospec/bin"
    autospec_env_file="$HOME/.autospec/env"
    autospec_env_line='. "$HOME/.autospec/env"'

    if [ "$DRY_RUN" -eq 1 ]; then
        info "  [dry-run] mkdir -p \"$autospec_bin_dir\""
        info "  [dry-run] write $autospec_env_file and source it from ~/.zshrc + ~/.bashrc"
        return 0
    fi

    mkdir -p "$autospec_bin_dir"
    cat > "$autospec_env_file" <<'EOF'
# autospec runtime command path
AUTOSPEC_BIN_DIR="$HOME/.autospec/bin"
case ":$PATH:" in
    *":$AUTOSPEC_BIN_DIR:"*) ;;
    *) export PATH="$AUTOSPEC_BIN_DIR:$PATH" ;;
esac
EOF

    for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
        [ -f "$rc" ] || touch "$rc"
        grep -qxF "$autospec_env_line" "$rc" || printf '%s\n' "$autospec_env_line" >> "$rc"
    done
    info "  PATH configured: $autospec_bin_dir via $autospec_env_file"
}

write_autonomous_operator_wrapper() {
    target="$1"
    subcommand="$2"

    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf '%s\n' 'set -eu'
        printf '%s\n' 'AUTOSPEC_WRAPPER_BIN_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"'
        printf '%s\n' 'case ":$PATH:" in'
        printf '%s\n' '    *":$AUTOSPEC_WRAPPER_BIN_DIR:"*) ;;'
        printf '%s\n' '    *) PATH="$AUTOSPEC_WRAPPER_BIN_DIR:$PATH"; export PATH ;;'
        printf '%s\n' 'esac'
        if [ -n "$subcommand" ]; then
            printf '%s\n' 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous-launcher.sh" '"$subcommand"' "$@"'
        else
            printf '%s\n' 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous-launcher.sh" "$@"'
        fi
    } > "$target"
    chmod +x "$target"
}

autonomous_operator_wrapper_exec_target() {
    wrapper="$1"
    [ -f "$wrapper" ] || return 1
    sed -n 's/^exec "\([^"]*\)".*/\1/p; s/^exec \([^ "$][^ ]*\).*/\1/p' "$wrapper" | head -n 1
}

autonomous_operator_wrapper_needs_heal() {
    wrapper="$1"
    exec_target="$(autonomous_operator_wrapper_exec_target "$wrapper" 2>/dev/null || true)"
    [ -n "$exec_target" ] || return 1
    case "$exec_target" in
        /*)
            case "$exec_target" in
                "$HOME/.autospec/"*)
                    [ -e "$exec_target" ] || return 0
                    ;;
                *)
                    return 0
                    ;;
            esac
            ;;
    esac
    return 1
}

heal_autonomous_operator_wrappers() {
    autospec_bin_dir="$HOME/.autospec/bin"
    [ -d "$autospec_bin_dir" ] || return 0

    healed=0
    for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
        target="$autospec_bin_dir/$command"
        [ -f "$target" ] || continue
        if autonomous_operator_wrapper_needs_heal "$target"; then
            old_target="$(autonomous_operator_wrapper_exec_target "$target" 2>/dev/null || true)"
            subcommand="${command#autospec-autonomous-}"
            if [ "$subcommand" = "$command" ]; then
                subcommand=""
            fi
            write_autonomous_operator_wrapper "$target" "$subcommand"
            info "heal_autonomous_operator_wrappers: healed $target (old exec target: ${old_target:-unknown})"
            healed=$((healed + 1))
        fi
    done

    if [ "$healed" -gt 0 ]; then
        info "heal_autonomous_operator_wrappers: healed $healed autonomous wrapper(s)"
    fi
}

install_autonomous_operator_commands() {
    autospec_bin_dir="$HOME/.autospec/bin"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "  [dry-run] install autospec-autonomous command wrappers in $autospec_bin_dir"
        return 0
    fi

    mkdir -p "$autospec_bin_dir"
    heal_autonomous_operator_wrappers
    for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
        target="$autospec_bin_dir/$command"
        subcommand="${command#autospec-autonomous-}"
        if [ "$subcommand" = "$command" ]; then
            subcommand=""
        fi
        write_autonomous_operator_wrapper "$target" "$subcommand"
    done
    info "  command wrappers installed: $autospec_bin_dir/autospec-autonomous*"
}

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
    if [ "$USE_SYMLINK" -eq 1 ]; then
        err "--symlink requires running from a checkout of the autospec repo (no local files to symlink to)."
        exit 2
    fi
    fetch_source_files
fi

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

info ""
info "Runtime command PATH:"
ensure_autospec_bin_path

info ""
info "Shared autospec helper scripts:"
install_shared_scripts

info ""
info "Autonomous helper scripts:"
install_autonomous_scripts

info ""
info "Runtime lib files (scripts/lib/):"
install_lib_files

info ""
info "Autonomous command wrappers:"
install_autonomous_operator_commands

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
    info "  Claude Code:  /${SKILL_NAME} <feature description>"
    info "  OpenCode:     @${SKILL_NAME} <feature description>"
    info "  Codex CLI:    /${SKILL_NAME} <feature description>"
fi

exit 0
