#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/fs.sh — Filesystem snapshot driver
#
# Usage: fs.sh <paths_csv> <sample_files_per_dir> <out_dir>
#
# Copies each path in paths_csv into <out_dir>, preserving directory structure,
# but capping files per directory to sample_files_per_dir.
#
# Exit codes:
#   0 = ok
#   1 = fatal
#   2 = refuse-to-run

set -euo pipefail

PATHS_CSV="${1:-}"
SAMPLE_FILES_PER_DIR="${2:-50}"
OUT_DIR="${3:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$PATHS_CSV" ]; then
    printf 'fs.sh: fatal: PATHS_CSV argument required\n' >&2
    exit 1
fi

if [ -z "$OUT_DIR" ]; then
    printf 'fs.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

if ! [[ "$SAMPLE_FILES_PER_DIR" =~ ^[0-9]+$ ]] || [ "$SAMPLE_FILES_PER_DIR" -lt 1 ]; then
    printf 'fs.sh: fatal: SAMPLE_FILES_PER_DIR must be a positive integer, got: %s\n' "$SAMPLE_FILES_PER_DIR" >&2
    exit 1
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

TOTAL_FILES=0
TOTAL_DIRS=0

# ── Process each path ─────────────────────────────────────────────────────────
IFS=',' read -ra PATHS <<< "$PATHS_CSV"
for src_path in "${PATHS[@]}"; do
    src_path="${src_path// /}"  # trim whitespace
    [ -z "$src_path" ] && continue

    if [ ! -e "$src_path" ]; then
        printf 'fs.sh: refuse-to-run: path does not exist: %s\n' "$src_path" >&2
        exit 2
    fi

    printf 'fs.sh: snapshotting path %s (cap %d files/dir)...\n' "$src_path" "$SAMPLE_FILES_PER_DIR" >&2

    if [ -f "$src_path" ]; then
        # Single file — copy directly
        dest_dir="${OUT_DIR}/$(dirname "$src_path")"
        mkdir -p "$dest_dir"
        cp "$src_path" "$dest_dir/" 2>&1 || {
            printf 'fs.sh: fatal: failed to copy %s\n' "$src_path" >&2
            exit 1
        }
        TOTAL_FILES=$((TOTAL_FILES + 1))
        continue
    fi

    # Directory — walk and cap files per sub-directory
    while IFS= read -r -d '' dir; do
        # Compute relative path for destination
        rel_dir="${dir#$src_path}"
        rel_dir="${rel_dir#/}"
        dest_dir="${OUT_DIR}/${src_path#/}/${rel_dir}"
        mkdir -p "$dest_dir"
        TOTAL_DIRS=$((TOTAL_DIRS + 1))

        # List files in this dir only (non-recursive), cap to sample_files_per_dir
        file_count=0
        while IFS= read -r -d '' file; do
            [ "$file_count" -ge "$SAMPLE_FILES_PER_DIR" ] && break
            cp "$file" "$dest_dir/" 2>&1 || {
                printf 'fs.sh: warning: failed to copy %s\n' "$file" >&2
                continue
            }
            file_count=$((file_count + 1))
            TOTAL_FILES=$((TOTAL_FILES + 1))
        done < <(find "$dir" -maxdepth 1 -type f -print0 | sort -z)
    done < <(find "$src_path" -type d -print0 | sort -z)
done

printf 'fs.sh: snapshot complete — %d file(s) from %d dir(s) → %s\n' "$TOTAL_FILES" "$TOTAL_DIRS" "$OUT_DIR" >&2
