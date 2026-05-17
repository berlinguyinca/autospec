#!/usr/bin/env bash
# Reusable install helpers. Source this file; do not execute.

# Idempotently merge a fenced block into a target file.
# Usage: merge_marked_block <file> <marker-name> <content>
# Writes:
#   <!-- marker-name -->
#   content
#   <!-- /marker-name -->
# If the block exists, it is replaced in place. If not, it is appended.
merge_marked_block() {
    local file="$1"
    local marker="$2"
    local content="$3"
    local start="<!-- ${marker} -->"
    local end="<!-- /${marker} -->"
    local tmp
    tmp=$(mktemp)

    [[ -f "$file" ]] || touch "$file"

    if grep -q "$start" "$file"; then
        awk -v s="$start" -v e="$end" -v c="$content" '
            $0 == s { print; print c; in_block=1; next }
            $0 == e { in_block=0; print; next }
            !in_block { print }
        ' "$file" > "$tmp"
    else
        cat "$file" > "$tmp"
        {
            [[ -s "$tmp" ]] && echo ""
            echo "$start"
            echo "$content"
            echo "$end"
        } >> "$tmp"
    fi

    mv "$tmp" "$file"
}

# Check if a command exists on PATH.
# Usage: command_present <name>
command_present() {
    command -v "$1" >/dev/null 2>&1
}

# Ensure a line is present in a file. Idempotent.
# Usage: ensure_line_in_file <file> <line>
ensure_line_in_file() {
    local file="$1"
    local line="$2"
    [[ -f "$file" ]] || touch "$file"
    grep -qxF "$line" "$file" || echo "$line" >> "$file"
}
