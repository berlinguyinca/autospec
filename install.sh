#!/usr/bin/env bash
# install.sh — top-level orchestrator for the autospec multi-skill suite.
#
# Delegates to per-skill installers under skills/<skill>/install.sh. Use this
# script to install every skill into every harness in one call. The per-skill
# installers remain standalone-callable.
#
# Usage:
#   ./install.sh                                 # install every skill into every harness
#   ./install.sh --skill autospec                # only one skill
#   ./install.sh --harness claude                # only one harness
#   ./install.sh --skill all --harness all       # explicit (same as default)
#   ./install.sh --skill autospec-run --update   # idempotent re-install
#   ./install.sh --help                          # show this help
#
# Flags:
#   --skill   one of: autospec | autospec-release | autospec-split | autospec-define | autospec-run | autospec-review | autospec-classify | autospec-listen | autospec-story | autospec-stop | autospec-sweep | autospec-design | autospec-fleet | autospec-qa | autospec-playwright | all
#             (default: all)
#   --harness one of: claude | opencode | codex | all
#             (default: all)
#   --update  forwarded to each per-skill installer; idempotent overwrite.
#   --dry-run forwarded to each per-skill installer; print actions, write nothing.
#
# Honors:
#   AUTOSPEC_NO_STAR_PROMPT=1  skip the optional GitHub star prompt.
#   AUTOSPEC_SKIP_SYSTEM_TOOLS=1  skip best-effort CLI dependency installs.
#   AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1  skip peer ecosystem bootstrap.
#
# Exits non-zero on any sub-installer failure; reports per-pair status.

set -eu

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
SKILLS_DIR="$REPO_ROOT/skills"

# shellcheck source=scripts/lib/install-helpers.sh
. "$REPO_ROOT/scripts/lib/install-helpers.sh"

TURBO_REPO_DIR="${TURBO_REPO_DIR:-$HOME/.turbo/repo}"
# autospec installs and updates track the berlinguyinca/turbo fork (carries the
# autospec-tuned skill set). Override with TURBO_REMOTE to point elsewhere.
TURBO_REMOTE="${TURBO_REMOTE:-https://github.com/berlinguyinca/turbo.git}"
CLAUDE_SKILLS_DIR="$HOME/.claude/skills"
SUPERPOWERS_REPO_DIR="${SUPERPOWERS_REPO_DIR:-$HOME/.codex/superpowers}"
SUPERPOWERS_REMOTE="${SUPERPOWERS_REMOTE:-https://github.com/obra/superpowers.git}"
SUPERPOWERS_CODEX_SKILLS_DIR="${SUPERPOWERS_CODEX_SKILLS_DIR:-$HOME/.agents/skills}"
SUPERPOWERS_OPENCODE_PLUGIN="${SUPERPOWERS_OPENCODE_PLUGIN:-superpowers@git+https://github.com/obra/superpowers.git}"
OPENCODE_CONFIG_ROOT="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
AUTOSPEC_SYSTEM_TOOLS="${AUTOSPEC_SYSTEM_TOOLS:-git bash curl jq yq gh node npm bun bats codex claude opencode omx omc oh-my-opencode mempalace ajv}"
OH_MY_CODEX_PACKAGE="${OH_MY_CODEX_PACKAGE:-oh-my-codex}"
OH_MY_OPENCODE_PACKAGE="${OH_MY_OPENCODE_PACKAGE:-oh-my-opencode}"
OH_MY_CLAUDE_PACKAGE="${OH_MY_CLAUDE_PACKAGE:-oh-my-claude-sisyphus}"

# Auto-discover skills from the repo by walking skills/*/install.sh. Earlier
# this was a hardcoded list, which silently dropped any new skill that didn't
# remember to update install.sh (autospec-refine via #674 and
# autospec-continue via #701 both shipped this way; operators never saw them).
ALL_SKILLS="$(
    for d in "$SKILLS_DIR"/*/; do
        [ -f "$d/install.sh" ] || continue
        basename "$d"
    done | sort | tr '\n' ' '
)"
# Drop the trailing space from the join above.
ALL_SKILLS="${ALL_SKILLS% }"
ALL_HARNESSES="claude opencode codex"

SKILL_ARG="all"
HARNESS_ARG="all"
UPDATE=0
DRY_RUN=0
DISABLE_AUTO_ROLLOVER=0
HOOK_MODE_ARG=""

err()  { printf 'error: %s\n' "$*" >&2; }
warn() { printf 'warn:  %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'
}

run_or_report() {
    description="$1"; shift
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] $description: $*"
    else
        info "$description"
        "$@"
    fi
}

offer_gitignore() {
    # Offer to ignore autospec runtime scratch files while keeping the tracked
    # project config `.autospec/autospec.yml` visible to git.
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 0
    repo_root="$(git rev-parse --show-toplevel)"
    gitignore="$repo_root/.gitignore"
    entry=".autospec/*"
    config_exception="!.autospec/autospec.yml"

    if [ -f "$gitignore" ] && grep -qxF "$entry" "$gitignore" && grep -qxF "$config_exception" "$gitignore"; then
        return 0
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] offer_gitignore: would add $entry and $config_exception to $gitignore"
        return 0
    fi

    if [ "${AUTOSPEC_AUTO_YES:-0}" = "1" ]; then
        ensure_line_in_file "$gitignore" "$entry"
        ensure_line_in_file "$gitignore" "$config_exception"
        info "offer_gitignore: added $entry and $config_exception to $gitignore"
        return 0
    fi

    # Only prompt on real interactive TTYs; otherwise skip silently.
    [ -t 0 ] && [ -t 1 ] || return 0

    printf "offer_gitignore: ignore autospec runtime files but track .autospec/autospec.yml in %s? [y/N] " "$gitignore"
    read -r reply || return 0
    case "$reply" in
        y|Y|yes|YES|Yes)
            ensure_line_in_file "$gitignore" "$entry"
            ensure_line_in_file "$gitignore" "$config_exception"
            info "offer_gitignore: added $entry and $config_exception to $gitignore"
            ;;
        *)
            info "offer_gitignore: skipped"
            ;;
    esac
}

merge_claude_md() {
    claude_md="$HOME/.claude/CLAUDE.md"
    block_file="$REPO_ROOT/scripts/lib/claude-md-block.txt"
    if [ ! -f "$block_file" ]; then
        warn "merge_claude_md: $block_file missing; skipping CLAUDE.md merge"
        return 0
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] merge_claude_md: would update <!-- autospec-block --> in $claude_md"
        return 0
    fi

    mkdir -p "$(dirname "$claude_md")"
    content="$(cat "$block_file")"
    merge_marked_block "$claude_md" "autospec-block" "$content"
    info "merge_claude_md: updated $claude_md"
}

check_codex() {
    if command_present codex; then
        info "check_codex: codex CLI present ($(command -v codex))"
        return 0
    fi
    info "check_codex: codex CLI NOT found on PATH."
    info "  Phase 4 peer-review will skip gracefully until codex is installed."
    info "  Install: see https://github.com/openai/codex (or your package manager)."
    return 0
}

ensure_system_tools() {
    if [ "${AUTOSPEC_SKIP_SYSTEM_TOOLS:-0}" = "1" ]; then
        info "ensure_system_tools: skipped by AUTOSPEC_SKIP_SYSTEM_TOOLS=1"
        return 0
    fi
    ensure_tool="$REPO_ROOT/skills/autospec-shared/scripts/ensure-tool.sh"
    if [ ! -f "$ensure_tool" ]; then
        warn "ensure_system_tools: $ensure_tool missing; skipping"
        return 0
    fi
    for tool in $AUTOSPEC_SYSTEM_TOOLS; do
        if [ "$DRY_RUN" -eq 1 ]; then
            info "[dry-run] ensure_system_tools: would ensure $tool"
        else
            bash "$ensure_tool" "$tool" || true
        fi
    done
}

install_npm_ecosystem_package() {
    label="$1"
    package_name="$2"
    command_name="$3"
    skip_var="$4"

    eval "skip_val=\${${skip_var}:-0}"
    if [ "$skip_val" = "1" ]; then
        info "$label: skipped by $skip_var=1"
        return 0
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] $label: would npm install -g $package_name when $command_name is missing or --update is set"
        return 0
    fi

    if command_present "$command_name" && [ "$UPDATE" -eq 0 ]; then
        info "$label: $command_name present ($(command -v "$command_name"))"
        return 0
    fi

    if ! command_present npm; then
        warn "$label: npm not found; install Node.js/npm or rerun after ensure_system_tools succeeds"
        return 0
    fi

    npm install -g "$package_name" >/dev/null 2>&1 \
        && info "$label: installed/updated $package_name" \
        || warn "$label: npm install -g $package_name failed; continuing"
}

bootstrap_superpowers_codex_link() {
    src="$SUPERPOWERS_REPO_DIR/skills"
    target="$SUPERPOWERS_CODEX_SKILLS_DIR/superpowers"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] bootstrap_superpowers: would expose $src at $target"
        return 0
    fi

    [ -d "$src" ] || { warn "bootstrap_superpowers: $src missing; cannot expose Codex skills"; return 0; }
    mkdir -p "$SUPERPOWERS_CODEX_SKILLS_DIR"

    if [ -L "$target" ]; then
        ln -sfn "$src" "$target" \
            && info "bootstrap_superpowers: refreshed Codex skills symlink at $target" \
            || warn "bootstrap_superpowers: could not refresh $target"
    elif [ ! -e "$target" ]; then
        ln -s "$src" "$target" 2>/dev/null \
            && info "bootstrap_superpowers: linked Codex skills at $target" \
            || { cp -R "$src" "$target" 2>/dev/null \
                && info "bootstrap_superpowers: copied Codex skills to $target (symlink unavailable)" \
                || warn "bootstrap_superpowers: could not link or copy Codex skills to $target"; }
    else
        info "bootstrap_superpowers: existing $target left untouched"
    fi
}

bootstrap_superpowers_opencode_plugin() {
    config_file="$OPENCODE_CONFIG_ROOT/opencode.json"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] bootstrap_superpowers: would ensure OpenCode plugin $SUPERPOWERS_OPENCODE_PLUGIN in $config_file"
        return 0
    fi

    if ! command_present node; then
        warn "bootstrap_superpowers: node not found; cannot update $config_file"
        return 0
    fi

    mkdir -p "$OPENCODE_CONFIG_ROOT"
    if node - "$config_file" "$SUPERPOWERS_OPENCODE_PLUGIN" <<'NODE'
const fs = require('fs');
const path = require('path');
const file = process.argv[2];
const plugin = process.argv[3];
let config = {};
if (fs.existsSync(file)) {
  try {
    config = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (error) {
    console.error(`invalid JSON in ${file}: ${error.message}`);
    process.exit(2);
  }
}
const plugins = Array.isArray(config.plugin)
  ? config.plugin
  : (typeof config.plugin === 'string' ? [config.plugin] : []);
if (!plugins.includes(plugin)) {
  plugins.push(plugin);
}
config.plugin = plugins;
fs.mkdirSync(path.dirname(file), { recursive: true });
fs.writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`);
NODE
    then
        info "bootstrap_superpowers: ensured OpenCode plugin in $config_file"
    else
        warn "bootstrap_superpowers: could not update $config_file"
    fi
}

bootstrap_superpowers() {
    if [ "${AUTOSPEC_SKIP_SUPERPOWERS:-0}" = "1" ]; then
        info "bootstrap_superpowers: skipped by AUTOSPEC_SKIP_SUPERPOWERS=1"
        return 0
    fi

    if [ -d "$SUPERPOWERS_REPO_DIR/.git" ]; then
        info "bootstrap_superpowers: pulling obra/superpowers at $SUPERPOWERS_REPO_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            git -C "$SUPERPOWERS_REPO_DIR" pull --ff-only 2>/dev/null \
                || warn "bootstrap_superpowers: pull failed (offline or local changes); using cached superpowers"
        fi
    else
        info "bootstrap_superpowers: cloning obra/superpowers to $SUPERPOWERS_REPO_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            git clone --depth 1 "$SUPERPOWERS_REMOTE" "$SUPERPOWERS_REPO_DIR" 2>/dev/null \
                || { warn "bootstrap_superpowers: clone failed; superpowers will not be installed"; return 0; }
        fi
    fi

    bootstrap_superpowers_codex_link
    bootstrap_superpowers_opencode_plugin
}

run_peer_setup_commands() {
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] bootstrap_oh_my_codex: would run omx setup --scope user --skill-target codex-home"
        info "[dry-run] bootstrap_oh_my_opencode: would initialize oh-my-opencode only when config is missing"
        info "[dry-run] bootstrap_oh_my_claude: would run omc setup --quiet"
        return 0
    fi

    if command_present omx; then
        omx setup --scope user --skill-target codex-home >/dev/null 2>&1 \
            && info "bootstrap_oh_my_codex: refreshed OMX setup" \
            || warn "bootstrap_oh_my_codex: omx setup failed; continuing"
    fi
    if command_present oh-my-opencode; then
        if [ -f "$OPENCODE_CONFIG_ROOT/oh-my-openagent.json" ]; then
            info "bootstrap_oh_my_opencode: existing config left untouched"
        else
            oh-my-opencode install --no-tui --claude=no --openai=no --gemini=no --copilot=no --skip-auth >/dev/null 2>&1 \
                && info "bootstrap_oh_my_opencode: initialized OpenCode setup" \
                || warn "bootstrap_oh_my_opencode: non-interactive setup failed; package install still completed"
        fi
    fi
    if command_present omc; then
        omc setup --quiet >/dev/null 2>&1 \
            && info "bootstrap_oh_my_claude: refreshed OMC setup" \
            || warn "bootstrap_oh_my_claude: omc setup failed; continuing"
    fi
}

bootstrap_peer_ecosystems() {
    if [ "${AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP:-0}" = "1" ]; then
        info "bootstrap_peer_ecosystems: skipped by AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1"
        return 0
    fi

    bootstrap_superpowers
    install_npm_ecosystem_package "bootstrap_oh_my_codex" "$OH_MY_CODEX_PACKAGE" "omx" "AUTOSPEC_SKIP_OH_MY_CODEX"
    install_npm_ecosystem_package "bootstrap_oh_my_opencode" "$OH_MY_OPENCODE_PACKAGE" "oh-my-opencode" "AUTOSPEC_SKIP_OH_MY_OPENCODE"
    install_npm_ecosystem_package "bootstrap_oh_my_claude" "$OH_MY_CLAUDE_PACKAGE" "omc" "AUTOSPEC_SKIP_OH_MY_CLAUDE"
    run_peer_setup_commands
}

bootstrap_turbo() {
    if [ -d "$TURBO_REPO_DIR/.git" ]; then
        info "bootstrap_turbo: pulling berlinguyinca/turbo at $TURBO_REPO_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            # Converge an existing clone onto the autospec-managed remote so a repo
            # originally cloned from upstream switches to the fork on the next update.
            # (Idempotent; preserves any 'upstream' remote the user added for PRs.)
            git -C "$TURBO_REPO_DIR" remote set-url origin "$TURBO_REMOTE" 2>/dev/null || true
            # Tolerate pull failures (no remote configured, offline, etc.) — turbo
            # is a nice-to-have peer skill family; absence shouldn't block install.
            git -C "$TURBO_REPO_DIR" pull --ff-only 2>/dev/null \
                || warn "bootstrap_turbo: pull failed (no remote or offline); using cached turbo"
        fi
    else
        info "bootstrap_turbo: cloning berlinguyinca/turbo to $TURBO_REPO_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            git clone --depth 1 "$TURBO_REMOTE" "$TURBO_REPO_DIR" 2>/dev/null \
                || { warn "bootstrap_turbo: clone failed; turbo skills will not be installed"; return 0; }
        fi
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] bootstrap_turbo: would symlink turbo skills into $CLAUDE_SKILLS_DIR/"
        return 0
    fi

    turbo_skills_src=""
    for candidate in "$TURBO_REPO_DIR/claude/skills" "$TURBO_REPO_DIR/skills"; do
        if [ -d "$candidate" ]; then
            turbo_skills_src="$candidate"
            break
        fi
    done
    if [ -z "$turbo_skills_src" ]; then
        warn "bootstrap_turbo: no skills directory found under $TURBO_REPO_DIR; turbo layout changed?"
        return 0
    fi

    mkdir -p "$CLAUDE_SKILLS_DIR"
    linked=0
    skipped_dir=0
    cleaned_nested=0
    for skill_dir in "$turbo_skills_src"/*; do
        [ -d "$skill_dir" ] || continue
        skill_name="$(basename "$skill_dir")"
        target="$CLAUDE_SKILLS_DIR/$skill_name"
        # Clean up corruption from earlier `ln -sfn src dir` runs that placed a nested
        # symlink at <target>/<skill_name> (instead of replacing the directory).
        nested="$target/$skill_name"
        if [ -L "$nested" ] && [ "$(readlink "$nested")" = "$skill_dir" ]; then
            rm "$nested"
            cleaned_nested=$((cleaned_nested + 1))
        fi
        if [ -L "$target" ]; then
            # We previously created this symlink; refresh it (handles turbo path changes).
            ln -sfn "$skill_dir" "$target"
            linked=$((linked + 1))
        elif [ ! -e "$target" ]; then
            ln -sfn "$skill_dir" "$target"
            linked=$((linked + 1))
        else
            # Pre-existing real directory (likely installed by hand or another tool).
            # Do NOT replace — that would clobber the user's content. Skip silently;
            # users who want autospec to manage these can `rm -rf $target` and re-run.
            skipped_dir=$((skipped_dir + 1))
        fi
    done
    info "bootstrap_turbo: $linked turbo skills symlinked into $CLAUDE_SKILLS_DIR/"
    if [ "$skipped_dir" -gt 0 ]; then
        info "bootstrap_turbo: $skipped_dir pre-existing skill dirs left untouched (delete one + re-run to switch it to a turbo-managed symlink)"
    fi
    if [ "$cleaned_nested" -gt 0 ]; then
        info "bootstrap_turbo: cleaned $cleaned_nested nested symlinks from earlier broken runs"
    fi
}

maybe_prompt_star() {
    # Keep scripted installs quiet: no prompt during updates, CI, or when opted out.
    [ "$UPDATE" -eq 0 ] || return 0
    [ "$DRY_RUN" -eq 0 ] || return 0
    [ "${AUTOSPEC_NO_STAR_PROMPT:-0}" != "1" ] || return 0
    [ "${CI:-}" = "" ] || return 0
    command -v gh >/dev/null 2>&1 || return 0

    answer=""
    if { exec 3<>/dev/tty; } 2>/dev/null; then
        info ""
        printf 'Would you like to star https://github.com/berlinguyinca/autospec to support adoption? [y/N] ' >&3
        read -r answer <&3 || { exec 3>&-; return 0; }
        exec 3>&-
    else
        [ -t 0 ] && [ -t 1 ] || return 0
        info ""
        printf 'Would you like to star https://github.com/berlinguyinca/autospec to support adoption? [y/N] '
        read -r answer || return 0
    fi

    case "$answer" in
        y|Y|yes|YES|Yes)
            if gh api -X PUT /user/starred/berlinguyinca/autospec >/dev/null 2>&1; then
                info "Thanks — starred berlinguyinca/autospec."
            else
                warn "could not star berlinguyinca/autospec; continuing"
            fi
            ;;
        *)
            info "No problem — skipping GitHub star."
            ;;
    esac
}

_ROLLOVER_MARKER_START="# >>> autospec auto-rollover >>>"
_ROLLOVER_MARKER_END="# <<< autospec auto-rollover <<<"

prompt_user_for_auto_rollover() {
    # Skip prompt during updates, CI, dry-run, or explicit disable.
    [ "$UPDATE" -eq 0 ] || return 0
    [ "$DRY_RUN" -eq 0 ] || return 0
    [ "$DISABLE_AUTO_ROLLOVER" -eq 0 ] || return 0
    [ "${CI:-}" = "" ] || return 0

    info ""
    info "autospec auto-context-rollover"
    info "  Wraps the claude/codex/opencode commands so that autospec-session"
    info "  monitors context usage and compacts/hands-off automatically."
    info "  A guard block is added to ~/.bashrc, ~/.zshrc, and fish config."
    info "  You can remove it anytime with:  bash install.sh --disable-auto-rollover"
    info ""

    answer=""
    if { exec 3<>/dev/tty; } 2>/dev/null; then
        printf '  Enable auto-context-rollover? [y/N] ' >&3
        read -r answer <&3 || { exec 3>&-; return 0; }
        exec 3>&-
    else
        [ -t 0 ] && [ -t 1 ] || return 0
        printf '  Enable auto-context-rollover? [y/N] '
        read -r answer || return 0
    fi

    case "$answer" in
        y|Y|yes|YES|Yes)
            install_rollover_block
            ;;
        *)
            info "  Skipping auto-rollover setup."
            ;;
    esac

    info ""
    info "autospec Claude hook mode"
    info "  Instead of polling via tmux, hook mode fires on Claude's PreCompact event."
    info "  Use this if you prefer native Claude Code integration over a background daemon."
    info "  Equivalent to running:  bash install.sh --hook-mode claude"
    info ""

    hook_ans=""
    if { exec 3<>/dev/tty; } 2>/dev/null; then
        printf '  Enable Claude hook mode? (fires on PreCompact instead of polling) [y/N] ' >&3
        read -r hook_ans <&3 || { exec 3>&-; return 0; }
        exec 3>&-
    else
        [ -t 0 ] && [ -t 1 ] || return 0
        printf '  Enable Claude hook mode? (fires on PreCompact instead of polling) [y/N] '
        read -r hook_ans || return 0
    fi

    case "$hook_ans" in
        y|Y|yes|YES|Yes)
            install_hook_mode_claude
            ;;
        *)
            info "  Skipping Claude hook mode setup."
            ;;
    esac
}

# Install the autospec_context_monitor Python package into user-site so the
# tmux launcher (scripts/autospec-session) and the Claude PreCompact hook can
# both `python3 -m autospec_context_monitor` without ModuleNotFoundError.
# Gap g-002: prior installs accepted the rollover prompt but never installed
# the package — the daemon failed on first launch with
# `No module named autospec_context_monitor`.
install_context_monitor_pkg() {
    local pkg_dir
    pkg_dir="$(dirname "$0")/packages/autospec_context_monitor"
    if [ ! -d "$pkg_dir" ]; then
        info "  autospec_context_monitor: package dir not found ($pkg_dir); skipping pip install"
        return 0
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] would run: pip install --user -e $pkg_dir"
        return 0
    fi

    if ! command -v python3 >/dev/null 2>&1; then
        info "  autospec_context_monitor: python3 not on PATH; skipping pip install"
        return 0
    fi

    # Skip when already importable (idempotent re-runs avoid pip churn).
    if python3 -c "import autospec_context_monitor" 2>/dev/null; then
        info "  autospec_context_monitor: already installed"
        return 0
    fi

    info "  autospec_context_monitor: pip install --user -e $pkg_dir"
    # --break-system-packages handles PEP 668 (externally-managed-environment) on
    # Homebrew Python, Apt-managed Python 3.11+, etc.  It is a no-op on Python
    # installations that do not enforce the marker file.
    local pip_out
    pip_out=$(python3 -m pip install --user --quiet --break-system-packages -e "$pkg_dir" 2>&1) || {
        err "  autospec_context_monitor: pip install failed"
        printf '%s\n' "$pip_out" | grep -v -E '^(WARNING|$)' >&2 || true
        err "  Recovery: run 'python3 -m pip install --user --break-system-packages -e $pkg_dir'"
        return 1
    }

    if python3 -c "import autospec_context_monitor" 2>/dev/null; then
        info "  autospec_context_monitor: import OK"
    else
        warn "  autospec_context_monitor: pip install completed but module not importable"
        warn "  (rollover daemon will fail; check 'python3 -m pip --version' and user-site PATH)"
    fi
}

install_rollover_block() {
    # First, ensure the python package backing autospec-context-monitor is
    # installed in user-site so both the tmux launcher and the Claude
    # PreCompact hook can `python3 -m autospec_context_monitor`.
    install_context_monitor_pkg

    local bash_block
    bash_block="$_ROLLOVER_MARKER_START
export AUTOSPEC_AUTO_ROLLOVER=1
if [ \"\${AUTOSPEC_AUTO_ROLLOVER:-0}\" = \"1\" ] && command -v autospec-session >/dev/null 2>&1; then
    claude()   { autospec-session claude \"\$@\"; }
    codex()    { autospec-session codex \"\$@\"; }
    opencode() { autospec-session opencode \"\$@\"; }
fi
$_ROLLOVER_MARKER_END"

    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] || continue
        if grep -qF "$_ROLLOVER_MARKER_START" "$rc"; then
            info "  auto-rollover block already present in $rc (skipping)"
            continue
        fi
        printf '\n%s\n' "$bash_block" >> "$rc"
        info "  auto-rollover block added to $rc"
    done

    # Fish uses function syntax instead of shell function declarations.
    local fish_config="$HOME/.config/fish/config.fish"
    if [ -f "$fish_config" ]; then
        if grep -qF "$_ROLLOVER_MARKER_START" "$fish_config"; then
            info "  auto-rollover block already present in $fish_config (skipping)"
        else
            printf '\n%s\n' "$_ROLLOVER_MARKER_START
set -x AUTOSPEC_AUTO_ROLLOVER 1
if test \"\$AUTOSPEC_AUTO_ROLLOVER\" = \"1\"; and command -v autospec-session >/dev/null 2>&1
    function claude; autospec-session claude \$argv; end
    function codex; autospec-session codex \$argv; end
    function opencode; autospec-session opencode \$argv; end
end
$_ROLLOVER_MARKER_END" >> "$fish_config"
            info "  auto-rollover block added to $fish_config"
        fi
    fi
}

remove_rollover_block() {
    local removed=0
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] || continue
        if grep -qF "$_ROLLOVER_MARKER_START" "$rc"; then
            # Use a temp file approach for portability (macOS sed -i differs from GNU).
            local tmp
            tmp=$(mktemp)
            awk "
                /$_ROLLOVER_MARKER_START/{skip=1}
                !skip{print}
                /$_ROLLOVER_MARKER_END/{skip=0}
            " "$rc" > "$tmp" && mv "$tmp" "$rc"
            info "  auto-rollover block removed from $rc"
            removed=$((removed + 1))
        fi
    done

    local fish_config="$HOME/.config/fish/config.fish"
    if [ -f "$fish_config" ] && grep -qF "$_ROLLOVER_MARKER_START" "$fish_config"; then
        local tmp
        tmp=$(mktemp)
        # WARNING: fish uninstall uses awk to remove lines between marker comments.
        # If you have manually edited or removed the marker comments, this awk command
        # will leave stale autospec lines in config.fish.
        # Recovery: manually delete lines between '# >>> autospec auto-rollover >>>'
        # and '# <<< autospec auto-rollover <<<' in ~/.config/fish/config.fish.
        awk "
            /$_ROLLOVER_MARKER_START/{skip=1}
            !skip{print}
            /$_ROLLOVER_MARKER_END/{skip=0}
        " "$fish_config" > "$tmp" && mv "$tmp" "$fish_config"
        info "  auto-rollover block removed from $fish_config"
        removed=$((removed + 1))
    fi

    if [ "$removed" -eq 0 ]; then
        info "  auto-rollover: no block found to remove (already clean)"
    fi
}

# Install PreCompact + SessionStart hooks into ~/.claude/settings.json.
# Uses python3 to read-modify-write the JSON atomically (tempfile + mv).
# Idempotent: re-running writes the same values, leaving the file hash
# unchanged when the entries are already present.
install_hook_mode_claude() {
    local settings="$HOME/.claude/settings.json"
    mkdir -p "$(dirname "$settings")"
    [ -f "$settings" ] || printf '{}' > "$settings"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] install_hook_mode_claude: would merge hooks.PreCompact + hooks.SessionStart into $settings"
        return 0
    fi

    python3 - "$settings" <<'PYEOF'
import json, sys, tempfile, os, pathlib

settings_path = pathlib.Path(sys.argv[1])
data = json.loads(settings_path.read_text(encoding="utf-8") or "{}")

hooks = data.setdefault("hooks", {})

monitor_cmd = "python3 -m autospec_context_monitor --hook-event"

# Claude Code's hooks schema requires each array entry to be an object with a
# `hooks` list of {type, command} steps -- not a bare command string. Earlier
# installs appended bare strings, which `claude doctor` flags as invalid.
def merge(event, step_extra=None):
    cmd = "%s %s" % (monitor_cmd, event)
    entries = hooks.setdefault(event, [])
    # Drop any legacy bare-string form AND any prior monitor entry, so re-running
    # the installer self-heals configs already in the wild (e.g. upgrading
    # PreCompact to async + timeout). Re-runs stay idempotent: we always drop our
    # own entry and re-append an identical one.
    entries[:] = [
        e for e in entries
        if e != cmd
        and not (
            isinstance(e, dict)
            and any(h.get("command") == cmd for h in e.get("hooks", []))
        )
    ]
    step = {"type": "command", "command": cmd}
    if step_extra:
        step.update(step_extra)
    entries.append({"hooks": [step]})

# PreCompact fires synchronously during compaction and Claude Code blocks on it;
# make the monitor step non-blocking (async) and time-bounded so a slow or
# blocked notifier can never freeze the harness.
merge("PreCompact", {"async": True, "timeout": 10})
merge("SessionStart")

tmp = settings_path.with_suffix(".json.tmp")
tmp.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
tmp.replace(settings_path)
print(f"install_hook_mode_claude: merged PreCompact + SessionStart into {settings_path}")
PYEOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --skill)
            shift
            SKILL_ARG="${1:-}"
            ;;
        --skill=*)
            SKILL_ARG="${1#--skill=}"
            ;;
        --harness)
            shift
            HARNESS_ARG="${1:-}"
            ;;
        --harness=*)
            HARNESS_ARG="${1#--harness=}"
            ;;
        --update)
            UPDATE=1
            ;;
        --dry-run)
            DRY_RUN=1
            ;;
        --disable-auto-rollover)
            DISABLE_AUTO_ROLLOVER=1
            ;;
        --hook-mode)
            shift
            HOOK_MODE_ARG="${1:-}"
            ;;
        --hook-mode=*)
            HOOK_MODE_ARG="${1#--hook-mode=}"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "unknown arg: $1"
            usage >&2
            exit 2
            ;;
    esac
    shift
done

# Validate --skill against the auto-discovered ALL_SKILLS list (#705 follow-up).
# Previously the validator carried its own hardcoded skill names; the
# discovery from #705 fixed `ALL_SKILLS=` but the validator still rejected
# autospec-refine / autospec-continue.
_skill_valid=0
if [ "$SKILL_ARG" = "all" ]; then
    _skill_valid=1
else
    for _s in $ALL_SKILLS; do
        if [ "$SKILL_ARG" = "$_s" ]; then _skill_valid=1; break; fi
    done
fi
if [ "$_skill_valid" != "1" ]; then
    err "invalid --skill: $SKILL_ARG"
    err "must be one of: $(printf '%s' "$ALL_SKILLS" | tr ' ' '\n' | sed 's/^/  /'; printf '  all\n')"
    exit 2
fi
unset _skill_valid _s

# Validate --harness
case "$HARNESS_ARG" in
    all|claude|opencode|codex) ;;
    *)
        err "invalid --harness: $HARNESS_ARG"
        err "must be one of: claude | opencode | codex | all"
        exit 2
        ;;
esac

# Resolve skill list
if [ "$SKILL_ARG" = "all" ]; then
    SKILLS_TO_RUN="$ALL_SKILLS"
else
    SKILLS_TO_RUN="$SKILL_ARG"
fi

# Resolve harness list
if [ "$HARNESS_ARG" = "all" ]; then
    HARNESSES_TO_RUN="$ALL_HARNESSES"
else
    HARNESSES_TO_RUN="$HARNESS_ARG"
fi

failures=0
total=0

info ""
info "autospec suite installer"
info "  skills:   $SKILLS_TO_RUN"
info "  harness:  $HARNESSES_TO_RUN"
[ "$UPDATE" -eq 1 ] && info "  mode:     --update (idempotent overwrite)"
[ "$DRY_RUN" -eq 1 ] && info "  mode:     --dry-run (no changes written)"
info ""

pull_autospec() {
    # Only runs under --update. Fast-forwards the autospec checkout if it has a
    # remote tracking branch; otherwise leaves it alone with a warning.
    [ "$UPDATE" -eq 1 ] || return 0
    info "pull_autospec: fast-forwarding autospec checkout at $REPO_ROOT"
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] pull_autospec: would git pull --ff-only in $REPO_ROOT"
        return 0
    fi
    git -C "$REPO_ROOT" pull --ff-only 2>/dev/null \
        || warn "pull_autospec: git pull failed (offline or no tracking branch); continuing"
}

copy_shared_scripts() {
    # Copy skills/autospec-shared/scripts/** to $AUTOSPEC_SCRIPTS_DIR preserving +x bits.
    # Runs on every install (not just --update) so new scripts are always present.
    shared_scripts_src="$REPO_ROOT/skills/autospec-shared/scripts"
    if [ ! -d "$shared_scripts_src" ]; then
        warn "copy_shared_scripts: $shared_scripts_src not found; skipping"
        return 0
    fi

    autospec_scripts_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] copy_shared_scripts: would copy $shared_scripts_src/** to $autospec_scripts_dir/"
        return 0
    fi

    mkdir -p "$autospec_scripts_dir"
    # Use cp -R with a trailing /. to copy contents (not the directory itself).
    # Then restore executable bits for all .sh and .mjs files.
    cp -R "$shared_scripts_src/." "$autospec_scripts_dir/"
    # Restore +x on shell scripts and mjs files (cp may strip bits on some platforms).
    # -maxdepth 3 covers nested helper dirs (e.g. gen-docs/architecture.mjs) once they
    # land under $autospec_scripts_dir/<subdir>/<file>.
    find "$autospec_scripts_dir" -maxdepth 3 \( -name '*.sh' -o -name '*.mjs' \) -exec chmod +x {} \;
    info "copy_shared_scripts: copied shared scripts to $autospec_scripts_dir/"
}

copy_repo_scripts() {
    # Copy repo-root scripts/*.{sh,mjs,ps1} to $AUTOSPEC_SCRIPTS_DIR preserving +x bits.
    # Globs (does not enumerate) so new repo-root helper scripts ship automatically for
    # every harness. Excludes scripts/lib/ (install-time-only helpers) and never reaches
    # per-skill target-repo gate scripts (those live under skills/, not scripts/).
    repo_scripts_src="$REPO_ROOT/scripts"
    if [ ! -d "$repo_scripts_src" ]; then
        warn "copy_repo_scripts: $repo_scripts_src not found; skipping"
        return 0
    fi

    autospec_scripts_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] copy_repo_scripts: would copy $repo_scripts_src/*.{sh,mjs,ps1} to $autospec_scripts_dir/"
        return 0
    fi

    mkdir -p "$autospec_scripts_dir"
    # Copy only top-level script files (no recursion into scripts/lib/).
    for ext in sh mjs ps1; do
        for f in "$repo_scripts_src"/*."$ext"; do
            [ -e "$f" ] || continue
            cp "$f" "$autospec_scripts_dir/"
        done
    done
    # Restore +x on shell scripts and mjs files (cp may strip bits on some platforms).
    find "$autospec_scripts_dir" -maxdepth 1 \( -name '*.sh' -o -name '*.mjs' \) -exec chmod +x {} \;
    info "copy_repo_scripts: copied repo-root scripts to $autospec_scripts_dir/"
}

copy_runtime_skill_scripts() {
    # Ship the per-skill helper scripts that skill *surfaces* invoke at runtime via
    # ${AUTOSPEC_SCRIPTS_DIR}/<name> but that copy_shared_scripts()/copy_repo_scripts()
    # do not reach (they live under skills/<skill>/scripts/, not scripts/ or
    # skills/autospec-shared/scripts/). The per-skill installers also ship these, but a
    # top-level `install.sh` invocation must land every runtime reference on its own so a
    # single install is complete (issue #985). Explicit src->dest manifest (not a glob):
    # several skills reuse generic basenames (sweep run.sh/wizard.sh, autospec-test
    # wizard.sh), so a flat copy would collide; the sweep scripts also ship under
    # autospec-sweep-* renames. tests/ship-completeness.bats mirrors this manifest.
    autospec_scripts_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"

    # Each entry is "<repo-relative source>::<dest path relative to $AUTOSPEC_SCRIPTS_DIR parent>".
    # Most entries install flat into $AUTOSPEC_SCRIPTS_DIR itself (dest has no slash prefix).
    # autospec-doc entries install into the skills/ subtree so that the two-level relative
    # import in gen-audience-docs.mjs resolves correctly (see comment below).
    runtime_skill_scripts="\
skills/autospec-run/scripts/autospec-run-session-lock.sh::autospec-run-session-lock.sh \
skills/autospec-run/scripts/autospec-run-status.sh::autospec-run-status.sh \
skills/autospec-run/scripts/claim-issue.sh::claim-issue.sh \
skills/autospec-run/scripts/list-ready-issues.sh::list-ready-issues.sh \
skills/autospec-run/scripts/post-token-report.sh::post-token-report.sh \
skills/autospec-run/scripts/release-issue.sh::release-issue.sh \
skills/autospec-resume/scripts/resume-scan.sh::resume-scan.sh \
skills/autospec-doc/scripts/doc-orchestrator-entry.mjs::doc-orchestrator.mjs \
skills/autospec-sweep/scripts/run.sh::autospec-sweep-run.sh \
skills/autospec-sweep/scripts/wizard.sh::autospec-sweep-wizard.sh"

    # autospec-doc module closure: doc-orchestrator.mjs and its ES-module siblings
    # must all reside in the same directory (relative static imports: ./doc-config.mjs,
    # ./gen-llms-full.mjs, ./doc-style.mjs). Additionally, gen-audience-docs.mjs resolves
    # shared deps via path.resolve(__dirname, '../../autospec-shared/scripts'). That
    # two-level climb means the scripts must live at a path of the form:
    #   <base>/autospec-doc/scripts/   (so ../../ reaches <base>/)
    # with the shared scripts mirrored at <base>/autospec-shared/scripts/.
    # We use $autospec_scripts_dir/../skills/ as <base>, giving:
    #   ~/.autospec/skills/autospec-doc/scripts/       — the module closure
    #   ~/.autospec/skills/autospec-shared/scripts/    — the shared deps mirror
    # A delegating shim (doc-orchestrator-entry.mjs) is installed flat at
    # $autospec_scripts_dir/doc-orchestrator.mjs (above) for backward-compatible
    # ${AUTOSPEC_SCRIPTS_DIR} invocations; it re-execs the subtree orchestrator. A
    # flat copy of the real orchestrator can never resolve its two-level shared import.
    #
    # This list MUST contain doc-orchestrator.mjs's full transitive ./ import closure:
    #   doc-orchestrator -> doc-config, doc-scaffold, gen-llms-full, gen-audience-docs, doc-coverage
    #   doc-scaffold     -> doc-config
    #   gen-audience-docs -> doc-style (+ ../../autospec-shared/scripts)
    # Omitting any of these crashes the orchestrator at module-load (ERR_MODULE_NOT_FOUND).
    # tests/ship-completeness.bats enforces this against the static import graph.
    autospec_doc_scripts="\
skills/autospec-doc/scripts/doc-orchestrator.mjs \
skills/autospec-doc/scripts/doc-config.mjs \
skills/autospec-doc/scripts/doc-scaffold.mjs \
skills/autospec-doc/scripts/doc-coverage.mjs \
skills/autospec-doc/scripts/doc-style.mjs \
skills/autospec-doc/scripts/gen-audience-docs.mjs \
skills/autospec-doc/scripts/gen-llms-full.mjs \
skills/autospec-doc/scripts/verify-examples.mjs"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] copy_runtime_skill_scripts: would copy 10 per-skill runtime scripts to $autospec_scripts_dir/"
        info "[dry-run] copy_runtime_skill_scripts: would copy autospec-doc module closure to $(dirname "$autospec_scripts_dir")/skills/autospec-doc/scripts/"
        info "[dry-run] copy_runtime_skill_scripts: would mirror shared scripts to $(dirname "$autospec_scripts_dir")/skills/autospec-shared/scripts/"
        return 0
    fi

    mkdir -p "$autospec_scripts_dir"
    for entry in $runtime_skill_scripts; do
        src="$REPO_ROOT/${entry%%::*}"
        dest="$autospec_scripts_dir/${entry##*::}"
        if [ -f "$src" ]; then
            cp "$src" "$dest"
            case "$dest" in
                *.sh|*.mjs) chmod +x "$dest" ;;
            esac
        else
            warn "copy_runtime_skill_scripts: source not found: $src (skipping)"
        fi
    done

    # Install the full autospec-doc module closure into the two-level subtree so
    # gen-audience-docs.mjs's path.resolve(__dirname, '../../autospec-shared/scripts')
    # resolves to ~/.autospec/skills/autospec-shared/scripts/ at runtime.
    autospec_doc_dest="$(dirname "$autospec_scripts_dir")/skills/autospec-doc/scripts"
    mkdir -p "$autospec_doc_dest"
    for src_rel in $autospec_doc_scripts; do
        src="$REPO_ROOT/$src_rel"
        dest="$autospec_doc_dest/$(basename "$src_rel")"
        if [ -f "$src" ]; then
            cp "$src" "$dest"
            chmod +x "$dest"
        else
            warn "copy_runtime_skill_scripts: source not found: $src (skipping)"
        fi
    done

    # Mirror shared scripts alongside so the ../../autospec-shared/scripts import resolves.
    shared_scripts_src="$REPO_ROOT/skills/autospec-shared/scripts"
    shared_scripts_mirror="$(dirname "$autospec_scripts_dir")/skills/autospec-shared/scripts"
    if [ -d "$shared_scripts_src" ]; then
        mkdir -p "$shared_scripts_mirror"
        cp -R "$shared_scripts_src/." "$shared_scripts_mirror/"
        find "$shared_scripts_mirror" -maxdepth 3 \( -name '*.sh' -o -name '*.mjs' \) -exec chmod +x {} \;
    else
        warn "copy_runtime_skill_scripts: $shared_scripts_src not found; autospec-doc shared imports may fail"
    fi

    info "copy_runtime_skill_scripts: copied per-skill runtime scripts to $autospec_scripts_dir/"
}

# expand_skill_file <src> <dest>
#
# D2 block-expansion step (§5 D2, §6 version-skew rule).
# If <src> contains an autospec-block marker, pipe it through the REPO copy of
# expand-skill-blocks.sh and write the result to <dest>.  Files without markers
# copy byte-identically.  Fails closed: if the expander exits non-zero, emit a
# clear message and return non-zero without writing <dest>.  After a successful
# expansion the installed copy is re-checked for any residual marker; if found,
# the dest is removed and the function returns non-zero.
#
# Version-skew rule: the expander is ALWAYS resolved relative to this script's
# own REPO_ROOT — never from a previously-installed ~/.autospec/scripts copy.
#
# Bash 3.2-compatible: no associative arrays, no RETURN traps, if/then/fi for
# one-sided conditionals under set -e.
expand_skill_file() {
    _esf_src="$1"
    _esf_dest="$2"
    _esf_expander="$REPO_ROOT/scripts/expand-skill-blocks.sh"

    if grep -q '<!-- autospec-block:' "$_esf_src" 2>/dev/null; then
        if [ ! -x "$_esf_expander" ]; then
            err "expand_skill_file: expander not found or not executable: $_esf_expander"
            return 1
        fi
        if ! "$_esf_expander" "$_esf_src" > "$_esf_dest"; then
            err "expand_skill_file: expander failed for $_esf_src; aborting install of this file"
            rm -f "$_esf_dest"
            return 1
        fi
        # Post-install safety: ensure no unexpanded marker survived.
        if grep -q '<!-- autospec-block:' "$_esf_dest" 2>/dev/null; then
            err "expand_skill_file: installed copy still contains unexpanded marker: $_esf_dest"
            rm -f "$_esf_dest"
            return 1
        fi
    else
        cp "$_esf_src" "$_esf_dest"
    fi
}

copy_schemas() {
    # Copy repo-root schemas/*.json into $AUTOSPEC_SCHEMAS_DIR (default ~/.autospec/schemas/).
    # Mirrors copy_repo_scripts for the schemas/ directory. Runs on every install
    # (fresh + --update) so validate-qa-artifacts.sh can resolve schemas from the
    # installed location without requiring a repo checkout alongside the scripts.
    # Issue #856: install.sh never shipped schemas/, causing the installed validator
    # to fail with "missing ~/.autospec/schemas/*.json" for all non-checkout users.
    schemas_src="$REPO_ROOT/schemas"
    if [ ! -d "$schemas_src" ]; then
        warn "copy_schemas: $schemas_src not found; skipping"
        return 0
    fi

    autospec_schemas_dir="${AUTOSPEC_SCHEMAS_DIR:-$HOME/.autospec/schemas}"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] copy_schemas: would copy $schemas_src/*.json to $autospec_schemas_dir/"
        return 0
    fi

    mkdir -p "$autospec_schemas_dir"
    for f in "$schemas_src"/*.json; do
        [ -e "$f" ] || continue
        cp "$f" "$autospec_schemas_dir/"
    done
    info "copy_schemas: copied $(ls "$autospec_schemas_dir"/*.json 2>/dev/null | wc -l | tr -d ' ') schemas to $autospec_schemas_dir/"
}

copy_runtime_subdirs() {
    # Ship the runtime SUBDIRECTORIES under scripts/ that installed surfaces source or
    # exec at runtime via $SCRIPT_DIR/<subdir>. copy_repo_scripts() is maxdepth-1 and
    # ships only top-level scripts/*.sh, so without this step a clean install leaves
    # $AUTOSPEC_SCRIPTS_DIR/lib/ and .../explore-research/ empty — autospec-explore.sh
    # then dies sourcing lib/autospec-loop.sh and every researcher fails to resolve at
    # $SCRIPT_DIR/explore-research/. Same class of bug as copy_schemas (issue #856).
    #
    # scripts/lib/ is MIXED: install-time-only helpers (install-helpers.sh,
    # claude-md-block.txt) stay OUT of the runtime tree; only the runtime libs sourced
    # by installed scripts ship. scripts/explore-research/ is entirely runtime.
    autospec_scripts_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"

    # Runtime libs sourced/exec'd by installed scripts at $SCRIPT_DIR/lib/<name>.
    runtime_libs="autospec-loop.sh autospec-harness-detect.sh explore-internet-safety.sh extract-matchers.sh"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] copy_runtime_subdirs: would copy runtime libs + scripts/explore-research/ to $autospec_scripts_dir/"
        return 0
    fi

    mkdir -p "$autospec_scripts_dir/lib"
    for _lib in $runtime_libs; do
        _src="$REPO_ROOT/scripts/lib/$_lib"
        if [ -f "$_src" ]; then
            cp "$_src" "$autospec_scripts_dir/lib/$_lib"
            chmod +x "$autospec_scripts_dir/lib/$_lib"
        else
            warn "copy_runtime_subdirs: runtime lib not found: $_src (skipping)"
        fi
    done

    if [ -d "$REPO_ROOT/scripts/explore-research" ]; then
        mkdir -p "$autospec_scripts_dir/explore-research"
        cp -R "$REPO_ROOT/scripts/explore-research/." "$autospec_scripts_dir/explore-research/"
        find "$autospec_scripts_dir/explore-research" -maxdepth 1 -name '*.sh' -exec chmod +x {} \;
    else
        warn "copy_runtime_subdirs: $REPO_ROOT/scripts/explore-research not found; skipping"
    fi
    info "copy_runtime_subdirs: copied runtime libs + explore-research to $autospec_scripts_dir/"
}

# Integration bootstrap: pull autospec (if --update) + turbo, before per-skill installers run.
pull_autospec
copy_shared_scripts
copy_repo_scripts
copy_runtime_subdirs
copy_runtime_skill_scripts
copy_schemas
ensure_system_tools
bootstrap_peer_ecosystems
bootstrap_turbo
check_codex
merge_claude_md
offer_gitignore

for skill in $SKILLS_TO_RUN; do
    skill_installer="$SKILLS_DIR/$skill/install.sh"
    if [ ! -f "$skill_installer" ]; then
        err "missing per-skill installer: $skill_installer"
        failures=$((failures + 1))
        continue
    fi
    for harness in $HARNESSES_TO_RUN; do
        total=$((total + 1))
        info "==> $skill -> $harness"
        cmd="bash \"$skill_installer\" --harness \"$harness\""
        if [ "$UPDATE" -eq 1 ]; then
            cmd="$cmd --update"
        fi
        if [ "$DRY_RUN" -eq 1 ]; then
            cmd="$cmd --dry-run"
        fi
        if eval "$cmd"; then
            info "    OK: $skill ($harness)"
        else
            err  "    FAIL: $skill ($harness)"
            failures=$((failures + 1))
            info ""
            continue
        fi

        # D2 block-expansion pass (trio-wide, TOKR1-002, issue #1035): after the
        # per-skill installer copies files, expand EVERY installed trio-member
        # copy that originates from a marker-bearing source — not just SKILL.md.
        # Each harness installs different members:
        #   claude:   skills/<skill>/SKILL.md            (from SKILL.md)
        #   codex:    skills/<skill>/SKILL.md            (from SKILL.md)
        #             prompts/<skill>.md                 (from codex/prompt.md)
        #   opencode: agent/<skill>.md                   (from opencode/agent.md)
        # Without this, codex prompts/ and opencode agent/ copies shipped the
        # literal <!-- autospec-block:startup-self-update --> marker unexpanded,
        # silently dropping self-update on 2 of 3 harnesses.
        # Each (src,dest) pair is expanded independently and guarded by the
        # source carrying a marker; expand_skill_file fails closed (removes the
        # dest + returns non-zero) if the expander errors or a marker survives.
        # Version-skew rule §6: always invoke $REPO_ROOT/scripts/expand-skill-blocks.sh.
        # Bash 3.2-compatible: no associative arrays, if/then/fi under set -e.
        if [ "$DRY_RUN" -eq 0 ]; then
            _claude_dir="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
            _codex_dir="${CODEX_HOME:-$HOME/.codex}"
            _opencode_dir="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
            _skill_src_dir="$SKILLS_DIR/$skill"

            # Build the per-harness list of "src|dest" member pairs as newline
            # records (bash 3.2-safe; no arrays of arrays).
            _pairs=""
            case "$harness" in
                claude)
                    _pairs="$_skill_src_dir/SKILL.md|$_claude_dir/skills/$skill/SKILL.md"
                    ;;
                codex)
                    _pairs="$_skill_src_dir/SKILL.md|$_codex_dir/skills/$skill/SKILL.md
$_skill_src_dir/codex/prompt.md|$_codex_dir/prompts/$skill.md"
                    ;;
                opencode)
                    _pairs="$_skill_src_dir/opencode/agent.md|$_opencode_dir/agent/$skill.md"
                    ;;
            esac

            # Expand each member whose source carries a marker. Members without a
            # marker were already byte-copied by the per-skill installer and are
            # left untouched. The post-expand safety-grep inside expand_skill_file
            # covers every installed copy here, not just SKILL.md.
            _old_ifs="$IFS"
            IFS='
'
            for _pair in $_pairs; do
                [ -n "$_pair" ] || continue
                _src="${_pair%%|*}"
                _dest="${_pair#*|}"
                if [ -f "$_src" ] && grep -q '<!-- autospec-block:' "$_src" 2>/dev/null; then
                    if [ -f "$_dest" ]; then
                        if ! expand_skill_file "$_src" "$_dest"; then
                            err "  expand: failed for $skill ($harness) $_dest; marking pair failed"
                            failures=$((failures + 1))
                        else
                            info "  expanded: $_dest"
                        fi
                    fi
                fi
            done
            IFS="$_old_ifs"
            unset _pairs _pair _src _dest _claude_dir _codex_dir _opencode_dir _skill_src_dir _old_ifs
        fi

        info ""
    done
done

succeeded=$((total - failures))
info ""
info "Suite install summary: $succeeded/$total pairs OK ($failures failed)"

if [ "$failures" -gt 0 ]; then
    exit 1
fi
if [ "$DISABLE_AUTO_ROLLOVER" -eq 1 ]; then
    remove_rollover_block
    exit 0
fi
if [ "$HOOK_MODE_ARG" = "claude" ]; then
    install_hook_mode_claude
    exit 0
fi
prompt_user_for_auto_rollover
maybe_prompt_star
exit 0
