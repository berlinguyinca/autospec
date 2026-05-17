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
        run_or_report "bootstrap_turbo: pulling tobihagemann/turbo at $TURBO_REPO_DIR" \
            git -C "$TURBO_REPO_DIR" pull --ff-only
    else
        run_or_report "bootstrap_turbo: cloning tobihagemann/turbo to $TURBO_REPO_DIR" \
            git clone --depth 1 "$TURBO_REMOTE" "$TURBO_REPO_DIR"
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
    for skill_dir in "$turbo_skills_src"/*; do
        [ -d "$skill_dir" ] || continue
        skill_name="$(basename "$skill_dir")"
        ln -sfn "$skill_dir" "$CLAUDE_SKILLS_DIR/$skill_name"
    done
    info "bootstrap_turbo: turbo skills symlinked into $CLAUDE_SKILLS_DIR/"
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

# Integration bootstrap: pull turbo + symlink, before per-skill installers run.
bootstrap_turbo
check_codex

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
