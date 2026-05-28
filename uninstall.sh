#!/usr/bin/env bash
# uninstall.sh — top-level orchestrator for removing the autospec suite.
#
# Removes the install paths for every (skill, harness) pair selected. Idempotent:
# missing paths are not an error.
#
# Usage:
#   ./uninstall.sh                                 # remove every skill from every harness
#   ./uninstall.sh --skill autospec                # only one skill
#   ./uninstall.sh --harness claude                # only one harness
#   ./uninstall.sh --skill all --harness all       # explicit (same as default)
#   ./uninstall.sh --help                          # show this help
#
# Flags:
#   --skill   one of: autospec | autospec-release | autospec-split | autospec-define | autospec-run | autospec-review | autospec-classify | autospec-listen | autospec-story | autospec-stop | autospec-sweep | autospec-design | autospec-fleet | autospec-qa | all
#             (default: all)
#   --harness one of: claude | opencode | codex | all
#             (default: all)
#
# Honors:
#   CLAUDE_CONFIG_DIR    (default: $HOME/.claude)
#   OPENCODE_CONFIG_DIR  (default: $HOME/.config/opencode)
#   CODEX_HOME           (default: $HOME/.codex)

set -eu

ALL_SKILLS="autospec autospec-release autospec-split autospec-define autospec-run autospec-review autospec-classify autospec-listen autospec-story autospec-stop autospec-sweep autospec-design autospec-fleet autospec-qa"
ALL_HARNESSES="claude opencode codex"

SKILL_ARG="all"
HARNESS_ARG="all"

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
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

case "$SKILL_ARG" in
    all|autospec|autospec-release|autospec-split|autospec-define|autospec-run|autospec-review|autospec-classify|autospec-listen|autospec-story|autospec-stop|autospec-sweep|autospec-design|autospec-fleet|autospec-qa) ;;
    *)
        err "invalid --skill: $SKILL_ARG"
        exit 2
        ;;
esac
case "$HARNESS_ARG" in
    all|claude|opencode|codex) ;;
    *)
        err "invalid --harness: $HARNESS_ARG"
        exit 2
        ;;
esac

if [ "$SKILL_ARG" = "all" ]; then
    SKILLS_TO_RUN="$ALL_SKILLS"
else
    SKILLS_TO_RUN="$SKILL_ARG"
fi
if [ "$HARNESS_ARG" = "all" ]; then
    HARNESSES_TO_RUN="$ALL_HARNESSES"
else
    HARNESSES_TO_RUN="$HARNESS_ARG"
fi

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
OPENCODE_DIR="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"

removed=0
missing=0

remove_path() {
    target="$1"
    if [ -e "$target" ] || [ -L "$target" ]; then
        rm -rf "$target"
        info "  removed: $target"
        removed=$((removed + 1))
    else
        info "  (skip — not present): $target"
        missing=$((missing + 1))
    fi
}

info ""
info "autospec suite uninstaller"
info "  skills:   $SKILLS_TO_RUN"
info "  harness:  $HARNESSES_TO_RUN"
info ""

for skill in $SKILLS_TO_RUN; do
    info "==> $skill"
    for harness in $HARNESSES_TO_RUN; do
        case "$harness" in
            claude)
                remove_path "$CLAUDE_DIR/skills/$skill"
                ;;
            opencode)
                remove_path "$OPENCODE_DIR/agent/$skill.md"
                ;;
            codex)
                remove_path "$CODEX_DIR/prompts/$skill.md"
                ;;
        esac
    done
    info ""
done

info ""
info "Suite uninstall summary: removed=$removed, already-absent=$missing"
exit 0
