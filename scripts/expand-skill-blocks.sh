#!/usr/bin/env bash
# expand-skill-blocks.sh — canonical skill-block expander (autospec D2).
#
# Replaces single-source block markers in a skill/markdown file with the
# matching template body from templates/skill-blocks/, performing LITERAL
# {{KEY}} -> value substitution. There is NO use of the bash builtin that runs
# a string as code, and NO shell expansion of
# template or parameter content: a parameter value of $(rm -rf /) lands in the
# output as the literal text "$(rm -rf /)". Fails closed (non-zero) on an
# unknown block name or a missing template file. A file containing no markers
# is reproduced byte-identically (idempotent).
#
# Usage:
#   expand-skill-blocks.sh <file>              # expanded output to stdout
#   expand-skill-blocks.sh --in-place <file>   # rewrite <file> in place
#   expand-skill-blocks.sh -h | --help
#
# Marker grammar (one block per marker line):
#   <!-- autospec-block:<name> KEY=value KEY2=value2 -->
#
# Exit codes:
#   0  success
#   2  usage error (missing/extra args, unreadable input)
#   3  unknown block / missing template file (fail closed)
set -u

PROG=$(basename "$0")
# Resolve repo root from this script's location (scripts/ -> repo root).
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
TEMPLATE_DIR="$REPO_ROOT/templates/skill-blocks"

usage() {
    cat <<EOF
$PROG — expand autospec skill-block markers with literal {{KEY}} substitution.

Usage:
  $PROG <file>              Print expanded output to stdout (caller redirects).
  $PROG --in-place <file>   Rewrite <file> in place (atomic temp+mv).
  $PROG -h | --help         Show this help.

Behavior:
  - Each marker line  <!-- autospec-block:<name> KEY=value ... -->  is replaced
    by templates/skill-blocks/<name>.md, with every literal {{KEY}} replaced by
    its value. Substitution is a pure string replace with no code execution
    and no shell expansion. A value of \$(cmd) is emitted verbatim, never run.
  - Lines without a marker are passed through unchanged; a file with no markers
    is reproduced byte-identically (idempotent).
  - Unknown block name or missing template file => exit 3 (fail closed).
EOF
}

die() {  # die <code> <message>
    local code=$1; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# Marker regex: capture the block name and the (possibly empty) param string.
# Anchored to a line that is exactly an HTML comment marker (leading ws allowed).
MARKER_RE='^[[:space:]]*<!--[[:space:]]+autospec-block:([A-Za-z0-9_-]+)([^>]*)-->[[:space:]]*$'

expand_marker() {  # expand_marker <name> <param_string>  -> body on stdout
    local name=$1 params=$2
    local tmpl="$TEMPLATE_DIR/$name.md"
    if [ ! -f "$tmpl" ]; then
        die 3 "unknown block '$name' (no template at $tmpl); failing closed"
    fi
    local body
    body=$(cat "$tmpl")
    # Parse KEY=VALUE tokens from the param string. Splitting on IFS whitespace
    # in a controlled read loop keeps values inert — no command substitution is
    # ever performed on the token text.
    local tok key val
    for tok in $params; do
        case "$tok" in
            *=*)
                key=${tok%%=*}
                val=${tok#*=}
                # Literal string replace of {{KEY}} -> val. Both operands are
                # plain variables; bash pattern substitution does not re-parse
                # val, so $(...) inside val is treated as literal characters.
                body=${body//\{\{$key\}\}/$val}
                ;;
            '') : ;;  # stray whitespace token
            *)  : ;;   # ignore non KEY=VALUE tokens defensively
        esac
    done
    printf '%s\n' "$body"
}

run_expand() {  # run_expand <file>  -> expanded stream on stdout
    local file=$1
    [ -f "$file" ] || die 2 "input file not found: $file"
    [ -r "$file" ] || die 2 "input file not readable: $file"
    # Read line by line. Preserve every non-marker line verbatim. Using
    # IFS= and read -r keeps leading/trailing whitespace intact. We track
    # whether the final line had a trailing newline so a no-marker file is
    # reproduced byte-identically.
    local line had_trailing_nl=1
    while IFS= read -r line || { had_trailing_nl=0; [ -n "$line" ]; }; do
        if [[ "$line" =~ $MARKER_RE ]]; then
            local name=${BASH_REMATCH[1]}
            local params=${BASH_REMATCH[2]}
            expand_marker "$name" "$params"
        else
            if [ "$had_trailing_nl" -eq 0 ]; then
                # Last line lacked a trailing newline: emit without one.
                printf '%s' "$line"
            else
                printf '%s\n' "$line"
            fi
        fi
    done < "$file"
}

main() {
    local in_place=0 file=""
    while [ $# -gt 0 ]; do
        case "$1" in
            -h|--help) usage; exit 0 ;;
            --in-place) in_place=1; shift ;;
            --) shift; break ;;
            -*) die 2 "unknown option: $1" ;;
            *)  if [ -n "$file" ]; then die 2 "unexpected extra argument: $1"; fi
                file=$1; shift ;;
        esac
    done
    # Any remaining positional after `--`
    if [ $# -gt 0 ]; then
        if [ -n "$file" ]; then die 2 "unexpected extra argument: $1"; fi
        file=$1; shift
        [ $# -eq 0 ] || die 2 "unexpected extra argument: $1"
    fi
    [ -n "$file" ] || { usage >&2; die 2 "missing <file> argument"; }

    if [ "$in_place" -eq 1 ]; then
        local tmp
        tmp=$(mktemp "${TMPDIR:-/tmp}/expand-skill-blocks.XXXXXX") \
            || die 2 "cannot create temp file"
        # On any failure inside run_expand it exits non-zero (die), leaving the
        # original file untouched; clean up the temp.
        if run_expand "$file" > "$tmp"; then
            mv "$tmp" "$file"
        else
            local rc=$?
            rm -f "$tmp"
            exit "$rc"
        fi
    else
        run_expand "$file"
    fi
}

main "$@"
