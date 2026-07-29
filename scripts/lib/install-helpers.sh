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
    local content_file
    tmp=$(mktemp)
    content_file=$(mktemp)

    [ -f "$file" ] || touch "$file"
    printf '%s\n' "$content" > "$content_file"

    if grep -q "$start" "$file"; then
        # Replace existing block in place. Repairs unclosed blocks by emitting the
        # closing marker in END if the input had no matching close. Reads content
        # from a file (not awk -v) so multi-line content is preserved.
        awk -v s="$start" -v e="$end" -v cf="$content_file" '
            $0 == s {
                print
                while ((getline line < cf) > 0) print line
                close(cf)
                in_block=1
                next
            }
            $0 == e { in_block=0; print; next }
            !in_block { print }
            END { if (in_block) print e }
        ' "$file" > "$tmp"
    else
        cat "$file" > "$tmp"
        {
            [ -s "$tmp" ] && echo ""
            echo "$start"
            cat "$content_file"
            echo "$end"
        } >> "$tmp"
    fi

    # Inline cleanup. Avoid RETURN trap here: bash RETURN traps are shell-scoped
    # by default and leak into caller frames, where $tmp would be unset and trip
    # set -u in install.sh.
    if mv "$tmp" "$file"; then
        rm -f "$content_file"
        return 0
    fi
    rm -f "$tmp" "$content_file"
    return 1
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
    [ -f "$file" ] || touch "$file"
    grep -qxF "$line" "$file" || echo "$line" >> "$file"
}
