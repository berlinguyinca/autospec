#!/usr/bin/env bash
# go-cover.sh — convert Go coverprofile to canonical lcov on stdout.
#
# Usage: collect <coverprofile>
#   <coverprofile>: path to Go test coverage profile (produced by go test -coverprofile=...)
#
# Converts via gcov2lcov or a built-in awk converter.
# Exit 0 = ok, 1 = fatal.

set -eu

collect() {
    local coverprofile="${1:-coverage.out}"

    if [ ! -f "$coverprofile" ]; then
        printf 'go-cover: fatal: coverprofile not found: %s\n' "$coverprofile" >&2
        exit 1
    fi

    # Try gcov2lcov if available
    if command -v gcov2lcov >/dev/null 2>&1; then
        gcov2lcov -infile="$coverprofile" 2>/dev/null
        return 0
    fi

    # Built-in awk converter for Go coverprofile format:
    # mode: set
    # github.com/foo/bar/file.go:10.20,12.5 1 1
    awk '
    BEGIN { sf=""; }
    /^mode:/ { next }
    {
        # Parse: package/file.go:startLine.startCol,endLine.endCol numStmt count
        split($1, loc, ":");
        file = loc[1];
        # Get just the filename part after last "/"
        n = split(file, parts, "/");
        split(loc[2], lines, ",");
        split(lines[1], startpos, ".");
        split(lines[2], endpos, ".");
        start_line = startpos[1];
        end_line   = endpos[1];
        count      = $3;

        if (file != sf) {
            if (sf != "") print "end_of_record";
            print "SF:" file;
            sf = file;
        }
        for (l = start_line; l <= end_line; l++) {
            print "DA:" l "," count;
        }
    }
    END { if (sf != "") print "end_of_record"; }
    ' "$coverprofile"
}

collect "${1:-coverage.out}"
