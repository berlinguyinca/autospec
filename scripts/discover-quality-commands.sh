#!/usr/bin/env bash
# scripts/discover-quality-commands.sh — discover the pre-merge quality command
# for every language actually present in a repository.
#
# The Phase 4 final quality gate used to hard-code Rust, so a polyglot repo had
# its shell, JavaScript, Python and Go code merged unlinted. This script is the
# single table of language marker -> quality command; the gate loops over its
# output instead of branching on Cargo.toml alone.
#
# Default mode prints one `marker<TAB>command` line per present marker, in a
# stable table order.
#
# `--missing-tools` prints one `marker<TAB>command<TAB>language<TAB>tool` line
# for every required executable of a discovered command that is not on PATH.
# A marker present with its linter absent is never a silent pass: the gate
# fails closed with `rule=<language>-unavailable`.
#
# `AUTOSPEC_FINAL_QUALITY_COMMAND` overrides discovery entirely — the single row
# `override<TAB><command>` is emitted and no marker is inspected. Its tools are
# not introspected (the value is arbitrary shell), so `--missing-tools` is empty.

set -eu

usage() {
    printf '%s\n' \
        'Usage:' \
        '  discover-quality-commands.sh [--repo-root <dir>] [--missing-tools]' \
        '' \
        'Default output: <marker>\t<command>' \
        '--missing-tools output: <marker>\t<command>\t<language>\t<absent-tool>'
}

die() {
    printf 'discover-quality-commands: %s\n' "$*" >&2
    exit 2
}

# The one and only language table. Columns are marker, language, the executables
# the command needs on PATH, and the command itself. printf reuses the format
# string per group of four arguments, so a new language is one row, not a block.
quality_command_table() {
    printf '%s\t%s\t%s\t%s\n' \
        'Cargo.toml' 'rust' 'cargo' \
        'cargo clippy --workspace --all-targets -- -D warnings' \
        '*.sh' 'shell' 'shellcheck' \
        'find . -name "*.sh" -not -path "./.git/*" -not -path "./node_modules/*" -not -path "*/tests/fixtures/*" -print0 | xargs -0 shellcheck -S error' \
        'package.json' 'javascript' 'npm' \
        'npm run lint' \
        'pyproject.toml' 'python' 'ruff' \
        'ruff check' \
        'go.mod' 'go' 'go golangci-lint' \
        'go vet ./... && golangci-lint run'
}

# Resolved before `cd "$REPO_ROOT"` so a relative invocation still finds the
# detector module. Empty when it cannot be resolved (e.g. a stripped PATH);
# the marker check then degrades to "not present".
SCRIPT_DIR="$( { cd "$(dirname "${BASH_SOURCE[0]}")" && pwd; } 2>/dev/null )"

# A marker is either a glob (`*.sh`, satisfied by at least one matching file
# anywhere outside .git/node_modules) or a file at the repo root or in a
# workspace-member subdirectory.
marker_present() {
    case "$1" in
        \**)
            [ -n "$(find . -name "$1" -not -path './.git/*' \
                -not -path './node_modules/*' -print -quit 2>/dev/null)" ]
            ;;
        *)
            [ -f "$1" ] && return 0
            # A file marker outside the root resolves the same way the stack
            # detector counts it. The detector owns the one exclusion list
            # (skipped directories + fixture trees); PYTHONPATH imports it
            # instead of keeping a second copy that can drift. python3 is the
            # detector's own dependency, so its absence degrades to
            # "marker not present" rather than a crash.
            PYTHONPATH="${SCRIPT_DIR}${PYTHONPATH:+:$PYTHONPATH}" \
                python3 - "$1" 2>/dev/null <<'PYEOF'
import sys
from pathlib import Path
import autospec_autonomy_stack as stack

marker = sys.argv[1]
for path in Path(".").rglob(marker):
    if not path.is_file():
        continue
    rel = path.relative_to(Path(".")).as_posix()
    if stack.is_skipped(rel):
        continue
    if stack.FIXTURE_DIR_PARTS.intersection(rel.split("/")[:-1]):
        continue
    sys.exit(0)
sys.exit(1)
PYEOF
            ;;
    esac
}

REPO_ROOT="$(pwd)"
MODE="default"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --missing-tools) MODE="missing-tools"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
cd "$REPO_ROOT"

if [ -n "${AUTOSPEC_FINAL_QUALITY_COMMAND:-}" ]; then
    if [ "$MODE" = "default" ]; then
        printf 'override\t%s\n' "$AUTOSPEC_FINAL_QUALITY_COMMAND"
    fi
    exit 0
fi

quality_command_table | while IFS="$(printf '\t')" read -r marker lang tools cmd; do
    marker_present "$marker" || continue
    if [ "$MODE" = "missing-tools" ]; then
        for tool in $tools; do
            command -v "$tool" >/dev/null 2>&1 \
                || printf '%s\t%s\t%s\t%s\n' "$marker" "$cmd" "$lang" "$tool"
        done
    else
        printf '%s\t%s\n' "$marker" "$cmd"
    fi
done
