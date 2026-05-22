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
#   --skill   one of: autospec | autospec-split | autospec-define | autospec-run | autospec-review | autospec-classify | autospec-listen | autospec-story | autospec-stop | all
#             (default: all)
#   --harness one of: claude | opencode | codex | all
#             (default: all)
#   --update  forwarded to each per-skill installer; idempotent overwrite.
#   --dry-run forwarded to each per-skill installer; print actions, write nothing.
#
# Honors:
#   AUTOSPEC_NO_STAR_PROMPT=1  skip the optional GitHub star prompt.
#
# Exits non-zero on any sub-installer failure; reports per-pair status.

set -eu

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
SKILLS_DIR="$REPO_ROOT/skills"

# shellcheck source=scripts/lib/install-helpers.sh
. "$REPO_ROOT/scripts/lib/install-helpers.sh"

TURBO_REPO_DIR="${TURBO_REPO_DIR:-$HOME/.turbo/repo}"
TURBO_REMOTE="https://github.com/tobihagemann/turbo.git"
CLAUDE_SKILLS_DIR="$HOME/.claude/skills"

ALL_SKILLS="autospec autospec-split autospec-define autospec-run autospec-review autospec-classify autospec-listen autospec-story autospec-stop"
ALL_HARNESSES="claude opencode codex"

SKILL_ARG="all"
HARNESS_ARG="all"
UPDATE=0
DRY_RUN=0

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
    # Offer to add `.autospec/` to the current repo's .gitignore so the integration
    # scratch directory does not pollute git status. No-op outside a git repo.
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 0
    repo_root="$(git rev-parse --show-toplevel)"
    gitignore="$repo_root/.gitignore"
    entry=".autospec/"

    if [ -f "$gitignore" ] && grep -qxF "$entry" "$gitignore"; then
        return 0
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] offer_gitignore: would add $entry to $gitignore"
        return 0
    fi

    if [ "${AUTOSPEC_AUTO_YES:-0}" = "1" ]; then
        ensure_line_in_file "$gitignore" "$entry"
        info "offer_gitignore: added $entry to $gitignore"
        return 0
    fi

    # Only prompt on real interactive TTYs; otherwise skip silently.
    [ -t 0 ] && [ -t 1 ] || return 0

    printf "offer_gitignore: add '%s' to %s? [y/N] " "$entry" "$gitignore"
    read -r reply || return 0
    case "$reply" in
        y|Y|yes|YES|Yes)
            ensure_line_in_file "$gitignore" "$entry"
            info "offer_gitignore: added $entry to $gitignore"
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

bootstrap_turbo() {
    if [ -d "$TURBO_REPO_DIR/.git" ]; then
        info "bootstrap_turbo: pulling tobihagemann/turbo at $TURBO_REPO_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            # Tolerate pull failures (no remote configured, offline, etc.) — turbo
            # is a nice-to-have peer skill family; absence shouldn't block install.
            git -C "$TURBO_REPO_DIR" pull --ff-only 2>/dev/null \
                || warn "bootstrap_turbo: pull failed (no remote or offline); using cached turbo"
        fi
    else
        info "bootstrap_turbo: cloning tobihagemann/turbo to $TURBO_REPO_DIR"
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
    # Keep scripted installs quiet: no prompt during updates, CI, pipes, or when opted out.
    [ "$UPDATE" -eq 0 ] || return 0
    [ "${AUTOSPEC_NO_STAR_PROMPT:-0}" != "1" ] || return 0
    [ "${CI:-}" = "" ] || return 0
    [ -t 0 ] && [ -t 1 ] || return 0
    command -v gh >/dev/null 2>&1 || return 0

    info ""
    printf 'Would you like to star https://github.com/berlinguyinca/autospec to support adoption? [y/N] '
    read -r answer || return 0
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

# Validate --skill
case "$SKILL_ARG" in
    all|autospec|autospec-split|autospec-define|autospec-run|autospec-review|autospec-classify|autospec-listen|autospec-story|autospec-stop) ;;
    *)
        err "invalid --skill: $SKILL_ARG"
        err "must be one of: autospec | autospec-split | autospec-define | autospec-run | autospec-review | autospec-classify | autospec-listen | autospec-story | autospec-stop | all"
        exit 2
        ;;
esac

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
    # Restore +x on shell scripts and mjs files (cp may strip bits on some platforms)
    find "$autospec_scripts_dir" -maxdepth 2 \( -name '*.sh' -o -name '*.mjs' \) -exec chmod +x {} \;
    info "copy_shared_scripts: copied shared scripts to $autospec_scripts_dir/"
}

# Integration bootstrap: pull autospec (if --update) + turbo, before per-skill installers run.
pull_autospec
copy_shared_scripts
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
maybe_prompt_star
exit 0
