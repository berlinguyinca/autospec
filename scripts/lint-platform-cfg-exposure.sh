#!/usr/bin/env bash
# lint-platform-cfg-exposure.sh — report the count and location of
# `cfg(not(target_os = "..."))` blocks (issue #4173, invariant 2).
#
# This workspace is developed on Linux. Every `#[cfg(not(target_os = "linux"))]`
# block is compiled on Windows/macOS but has NO compiler on this host, so any
# API drift on the Linux path (e.g. `Ref::swap`, a wrong import) cannot be
# caught until the next non-Linux build. Invariant 2 of #4173 asks for exactly
# this: a lint that reports the count and location of the exposure, so it is
# visible rather than silent.
#
# RULE_ID: PLATFORM_CFG_EXPOSURE
#
# The detector is intentionally conservative. A real attribute always writes the
# os with plain quotes (`not(target_os = "linux")`), whereas a string literal
# that merely *mentions* a cfg snippet escapes its quotes
# (`not(target_os = \"linux\")`) and is skipped. A plain `target_os = "linux"`
# gate (no `not(...)`) is the OPPOSITE of exposure — that code IS compiled on
# Linux — and is likewise not reported.
#
# This is a VISIBILITY report, not a gate: it exits 0 in every case. It exists
# so the exposure is counted and located (and so a regression that adds a new
# untested non-Linux block is visible in the output), not to block the build.
#
# Usage:
#   lint-platform-cfg-exposure.sh [--develop-on OS] [--list] [PATH...]
#   lint-platform-cfg-exposure.sh --help
#
# Options:
#   --develop-on OS   treat OS as the develop-on platform (default: derived
#                     from the current host: Linux -> linux, etc.)
#   --list            only emit the aggregate counts (one line)
#   --help            this help
#   PATH...           one or more .rs files or directories to scan
#                     (default: crates/ under the repository root)
#
# Output (one line per `not(target_os = "...")` occurrence, then a summary):
#   PLATFORM_CFG_EXPOSURE:<path>:<line>: not(target_os = "X") [off develop-on (<devon>); not compiled on this host]
#   PLATFORM_CFG_EXPOSURE_SUMMARY: <total> not(target_os = ...) block(s) in <files> file(s); <offdev> off develop-on (<devon>)
#
# Exit code: always 0 (this is an exposure report, not a blocking gate).

set -eu

script_path="$0"
script_dir=$(cd "$(dirname "$script_path")" && pwd)
root_dir=$(cd "$script_dir/.." && pwd)

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$script_path"
}

die() { local code="$1"; shift; printf 'ERROR: %s\n' "$*" >&2; exit "$code"; }

develop_on=""
listing=0
paths=()
while [ $# -gt 0 ]; do
    case "$1" in
        --develop-on) shift; develop_on="$1" ;;
        --list) listing=1 ;;
        --help|-h) usage; exit 0 ;;
        -*) die 2 "unknown option: $1 (see --help)" ;;
        *) paths+=("$1") ;;
    esac
    shift
done

# Default develop-on platform: derive from the current host, the same way the
# workspace knows which side of the split it is on.
if [ -z "$develop_on" ]; then
    case "$(uname -s)" in
        Linux) develop_on="linux" ;;
        Darwin) develop_on="macos" ;;
        MINGW*|MSYS*|CYGWIN*) develop_on="windows" ;;
        *) develop_on="linux" ;;
    esac
fi
develop_on_lower="${develop_on,,}"

# The plain-quote form: a real `cfg(not(target_os = "X"))` attribute. The
# escaped-quote form inside a string literal (`\"X\"`) does NOT match, and a
# `target_os = "X"` gate without `not(...)` does NOT match either.
awk_prog=$(cat <<'AWK'
{
    line = $0
    if (match(line, /not\(target_os[ \t]*=[ \t]*"[a-z0-9_]+"/) > 0) {
        s = substr(line, RSTART, RLENGTH)
        sub(/not\(target_os[ \t]*=[ \t]*"/, "", s)
        sub(/"$/, "", s)
        printf "%s\t%d\t%s\n", FILE, NR, s
    }
}
AWK
)

# Build the file list: explicit PATH args (each a .rs file or a directory to
# recurse into), or the crates/ tree by default.
files=()
if [ ${#paths[@]} -gt 0 ]; then
    for p in "${paths[@]}"; do
        if [ -d "$p" ]; then
            while IFS= read -r _f; do files+=("$_f"); done < <(find "$p" -type f -name '*.rs' | LC_ALL=C sort)
        elif [ -f "$p" ]; then
            files+=("$p")
        else
            die 4 "no such file or directory: $p"
        fi
    done
else
    while IFS= read -r _f; do files+=("$_f"); done < <(find "$root_dir/crates" -type f -name '*.rs' | LC_ALL=C sort)
fi

# Collect every occurrence as: <relpath>\t<line>\t<os>
sites=""
for file in ${files[@]+"${files[@]}"}; do
    [ -n "$file" ] || continue
    rel="${file#"$root_dir"/}"
    [ "$rel" = "$file" ] && rel="$file"
    out=$(LC_ALL=C awk -v FILE="$rel" "$awk_prog" "$file") || die 3 "awk failed on $file"
    [ -n "$out" ] || continue
    sites="${sites}${out}
"
done

if [ "$listing" -eq 1 ]; then
    total=0
    files_with=0
    offdev=0
    if [ -n "$sites" ]; then
        total=$(printf '%s' "$sites" | wc -l | tr -d '[:space:]')
        files_with=$(printf '%s' "$sites" | awk -F'\t' '{print $1}' | sort -u | wc -l | tr -d '[:space:]')
        offdev=$(printf '%s' "$sites" | awk -F'\t' -v d="$develop_on_lower" '$3 == d { c++ } END { print c + 0 }')
    fi
    printf 'PLATFORM_CFG_EXPOSURE_SUMMARY: %s not(target_os = ...) block(s) in %s file(s); %s off develop-on (%s)\n' \
        "$total" "$files_with" "$offdev" "$develop_on"
    exit 0
fi

if [ -n "$sites" ]; then
    total=$(printf '%s' "$sites" | wc -l | tr -d '[:space:]')
    files_with=$(printf '%s' "$sites" | awk -F'\t' '{print $1}' | sort -u | wc -l | tr -d '[:space:]')
    offdev=$(printf '%s' "$sites" | awk -F'\t' -v d="$develop_on_lower" '$3 == d { c++ } END { print c + 0 }')
    # Human-readable report: one line per block; mark the ones gated off the
    # develop-on platform (the code with no compiler on this host).
    printf '%s' "$sites" | LC_ALL=C sort -t$'\t' -k1,1 -k2,2n | awk -F'\t' -v devon="$develop_on_lower" '
        {
            path = $1; line = $2; os = $3
            if (os == devon)
                printf "PLATFORM_CFG_EXPOSURE:%s:%s: not(target_os = \"%s\") [off develop-on (%s); not compiled on this host]\n", path, line, os, devon
            else
                printf "PLATFORM_CFG_EXPOSURE:%s:%s: not(target_os = \"%s\")\n", path, line, os
        }
    '
    printf 'PLATFORM_CFG_EXPOSURE_SUMMARY: %s not(target_os = ...) block(s) in %s file(s); %s off develop-on (%s)\n' \
        "$total" "$files_with" "$offdev" "$develop_on"
else
    printf 'PLATFORM_CFG_EXPOSURE_SUMMARY: 0 not(target_os = ...) block(s) in 0 file(s); 0 off develop-on (%s)\n' "$develop_on"
fi

exit 0
