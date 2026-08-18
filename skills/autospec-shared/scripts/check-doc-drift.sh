#!/usr/bin/env bash
# check-doc-drift.sh — Deterministic doc-drift gate.
#
# Usage:
#   check-doc-drift.sh --diff <git_diff_file>    # diff from file (or - for stdin)
#   check-doc-drift.sh --pr <N>                  # fetch PR diff via gh CLI
#   check-doc-drift.sh --working-tree            # diff against HEAD
#
# Exit codes:
#   0 = clean (no drift found)
#   1 = drift detected
#   2 = missing-scope (changed source files not covered by any doc scope)
#
# Stdout: gate JSON per spec §3b
# Stderr: diagnostic messages
#
# Requires: bash 3.2+, node (for scan-doc-scope.mjs), git
# Optional: gh CLI (for --pr mode)
#
# Environment:
#   AUTOSPEC_DOCS_DIR     — docs directory to scan (default: docs/ under repo root)
#   AUTOSPEC_REPO_ROOT    — repo root (default: git rev-parse --show-toplevel)
#   SCAN_DOC_SCOPE_MJS    — path to scan-doc-scope.mjs (auto-discovered)

set -euo pipefail

# ── Resolve paths ─────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ -z "${AUTOSPEC_REPO_ROOT:-}" ]; then
    AUTOSPEC_REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || git rev-parse --show-toplevel)"
fi

DOCS_DIR="${AUTOSPEC_DOCS_DIR:-${AUTOSPEC_REPO_ROOT}/docs}"
SCAN_MJS="${SCAN_DOC_SCOPE_MJS:-${SCRIPT_DIR}/scan-doc-scope.mjs}"

if [ ! -f "$SCAN_MJS" ]; then
    echo "check-doc-drift: cannot find scan-doc-scope.mjs; set SCAN_DOC_SCOPE_MJS" >&2
    exit 2
fi

# ── Argument parsing ──────────────────────────────────────────────────────────

MODE=""
DIFF_ARG=""
PR_NUM=""

while [ $# -gt 0 ]; do
    case "$1" in
        --diff)
            MODE="diff"
            DIFF_ARG="${2:-}"
            if [ -z "$DIFF_ARG" ]; then
                echo "check-doc-drift: --diff requires an argument" >&2; exit 2
            fi
            shift 2
            ;;
        --pr)
            MODE="pr"
            PR_NUM="${2:-}"
            if [ -z "$PR_NUM" ]; then
                echo "check-doc-drift: --pr requires a PR number" >&2; exit 2
            fi
            shift 2
            ;;
        --working-tree)
            MODE="working-tree"
            shift
            ;;
        --help|-h)
            cat <<'USAGE'
check-doc-drift.sh — Deterministic doc-drift gate (autospec-docs phase 2)

Usage:
  check-doc-drift.sh --diff <file>    Analyse a saved git diff file (- for stdin)
  check-doc-drift.sh --pr <N>         Fetch PR #N diff via gh CLI
  check-doc-drift.sh --working-tree   Check working-tree changes vs HEAD

Exit codes: 0=clean  1=drift  2=missing-scope-or-error
Stdout: JSON gate result per spec §3b
USAGE
            exit 0
            ;;
        *)
            echo "check-doc-drift: unknown option: $1" >&2
            exit 2
            ;;
    esac
done

if [ -z "$MODE" ]; then
    echo "check-doc-drift: one of --diff, --pr, --working-tree required" >&2
    exit 2
fi

# ── Temp directory for working files ─────────────────────────────────────────

WORK_DIR="$(mktemp -d /tmp/autospec-drift-work-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

# ── Collect diff text ─────────────────────────────────────────────────────────

PR_BODY=""

case "$MODE" in
    diff)
        if [ "$DIFF_ARG" = "-" ]; then
            cat > "$WORK_DIR/diff.txt"
        elif [ -f "$DIFF_ARG" ]; then
            cp "$DIFF_ARG" "$WORK_DIR/diff.txt"
        else
            echo "check-doc-drift: diff file not found: $DIFF_ARG" >&2
            exit 2
        fi
        ;;
    pr)
        if ! command -v gh >/dev/null 2>&1; then
            echo "check-doc-drift: gh CLI not found; install it or use --diff mode" >&2
            exit 2
        fi
        if ! gh pr diff "$PR_NUM" > "$WORK_DIR/diff.txt" 2>/dev/null; then
            echo "check-doc-drift: failed to fetch diff for PR #${PR_NUM}" >&2
            exit 2
        fi
        PR_BODY="$(gh pr view "$PR_NUM" --json body --jq .body 2>/dev/null || true)"
        ;;
    working-tree)
        git -C "$AUTOSPEC_REPO_ROOT" diff HEAD > "$WORK_DIR/diff.txt" 2>/dev/null || true
        if [ ! -s "$WORK_DIR/diff.txt" ]; then
            git -C "$AUTOSPEC_REPO_ROOT" diff HEAD --cached > "$WORK_DIR/diff.txt" 2>/dev/null || true
        fi
        ;;
esac

# Ensure diff file exists
touch "$WORK_DIR/diff.txt"

# ── docs: skip check ──────────────────────────────────────────────────────────

DOCS_SKIP=false
if [ -n "$PR_BODY" ]; then
    if echo "$PR_BODY" | grep -qiE '^docs:[[:space:]]*skip[[:space:]]*$'; then
        DOCS_SKIP=true
    fi
fi

# ── no-scope-config short-circuit ────────────────────────────────────────────
# If the repo has zero `<!-- autospec-doc-scope: ... -->` annotations anywhere
# in DOCS_DIR, the gate is not opted-in: every changed non-docs/ file would
# otherwise be reported as missing-scope (exit 2), false-blocking every PR that
# touches source. Treat this like a repo-wide `docs: skip` until annotations
# are added. The pattern mirrors SCOPE_OPEN_RE in scan-doc-scope.mjs.
NO_SCOPE_REPO=false
if [ -d "$DOCS_DIR" ]; then
    if ! grep -rlE '<!--[[:space:]]*autospec-doc-scope[[:space:]]*:' "$DOCS_DIR" --include='*.md' >/dev/null 2>&1; then
        NO_SCOPE_REPO=true
        echo "check-doc-drift: no autospec-doc-scope annotations in ${DOCS_DIR}; gate not opted-in, skipping" >&2
    fi
fi

# ── Extract changed files from diff ──────────────────────────────────────────

# Changed source files (not docs/, not tests/) — one per line in a temp file.
# tests/** is excluded: test files are internal QA artifacts and never require
# a corresponding doc-scope annotation.  Including them causes exit 2
# (missing-scope) on every test-only PR that touches a path not covered by any
# declared scope — a false signal that degrades the gate without providing real
# drift information (tests don't document user-facing surfaces).
grep -E '^\+\+\+ ' "$WORK_DIR/diff.txt" 2>/dev/null \
    | sed 's|^+++ b/||; s|^+++ /dev/null||' \
    | grep -v '^$' \
    | grep -v '^docs/' \
    | grep -v '^tests/' \
    | sort -u > "$WORK_DIR/changed_source.txt" || true

# Changed doc files — one per line in a temp file
grep -E '^\+\+\+ ' "$WORK_DIR/diff.txt" 2>/dev/null \
    | sed 's|^+++ b/||; s|^+++ /dev/null||' \
    | grep -v '^$' \
    | grep '^docs/' \
    | sort -u > "$WORK_DIR/changed_docs.txt" || true

# For each changed doc file, extract the changed line numbers
# Format: one file per temp file named by index
while IFS= read -r df; do
    [ -z "$df" ] && continue
    safe_name="$(echo "$df" | tr '/' '_')"
    awk -v target="$df" '
        /^\+\+\+ b\// { in_file = ($0 == "+++ b/" target); lineno = 0 }
        in_file && /^@@ / {
            s = $0; sub(/.*\+/, "", s); split(s, a, ","); lineno = a[1] - 1
        }
        in_file && /^\+/ && !/^\+\+\+/ { lineno++; print lineno }
        in_file && /^[ ]/ { lineno++ }
    ' "$WORK_DIR/diff.txt" > "$WORK_DIR/changedlines_${safe_name}.txt" 2>/dev/null || true
done < "$WORK_DIR/changed_docs.txt"

# ── Glob matching helper ──────────────────────────────────────────────────────

# Write glob matcher script to a temp file once (avoids bash 3.2 escaping issues)
GLOB_MATCH_JS="$WORK_DIR/glob-match.mjs"
cat > "$GLOB_MATCH_JS" << 'GLOBEOF'
#!/usr/bin/env node
// glob-match.mjs <file> <pattern>
// Exit 0 if file matches glob pattern, 1 otherwise.
// Supports: * (not /), ** (any path), ? (single char)
const f = process.argv[2];
const p = process.argv[3];
if (!f || !p) process.exit(1);
function globToRe(pat) {
    let re = '^';
    let i = 0;
    while (i < pat.length) {
        if (pat[i] === '*' && pat[i+1] === '*') {
            re += '.*';
            i += 2;
            if (pat[i] === '/') i++; // consume trailing slash
        } else if (pat[i] === '*') {
            re += '[^/]*';
            i++;
        } else if (pat[i] === '?') {
            re += '[^/]';
            i++;
        } else {
            re += pat[i].replace(/[.+^${}()|[\]\\]/g, '\\$&');
            i++;
        }
    }
    re += '$';
    return new RegExp(re);
}
process.exit(globToRe(p).test(f) ? 0 : 1);
GLOBEOF

glob_match_js() {
    local file="$1"
    local pattern="$2"
    node "$GLOB_MATCH_JS" "$file" "$pattern" 2>/dev/null
}

# ── Process all docs/*.md for scope ──────────────────────────────────────────

# Track which source files are covered by any scope (write to temp file)
touch "$WORK_DIR/covered_sources.txt"

# Drift entries (newline-delimited JSON objects)
touch "$WORK_DIR/drift_entries.txt"
touch "$WORK_DIR/drift_warn_entries.txt"
touch "$WORK_DIR/visual_stale_entries.txt"
touch "$WORK_DIR/example_stale_entries.txt"

if [ -d "$DOCS_DIR" ]; then
    # Find all .md files in docs dir
    find "$DOCS_DIR" -name '*.md' > "$WORK_DIR/doc_files.txt" 2>/dev/null || true

    while IFS= read -r docfile; do
        [ -z "$docfile" ] && continue
        rel_docfile="${docfile#${AUTOSPEC_REPO_ROOT}/}"

        # Parse scope entries
        scope_json="$(node "$SCAN_MJS" "$docfile" 2>/dev/null)" || {
            echo "check-doc-drift: scan-doc-scope failed for ${docfile}" >&2
            continue
        }

        entry_count="$(echo "$scope_json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
process.stdout.write(String(d.length));
" 2>/dev/null)" || entry_count=0

        [ "$entry_count" -eq 0 ] 2>/dev/null && continue

        idx=0
        while [ "$idx" -lt "$entry_count" ]; do
            # Extract fields for this entry
            entry_json="$(echo "$scope_json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
const e=d[$idx];
process.stdout.write(JSON.stringify({
  heading: e.heading_path||'',
  reason: e.reason||'',
  visual_glob: e.visual_glob||'',
  src_globs: e.src_globs||[],
  mismatch_action: e.mismatch_action||'hard_fail',
  byte_start: e.byte_range[0],
  byte_end: e.byte_range[1]
}));
" 2>/dev/null)"

            heading="$(echo "$entry_json" | node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).heading)" 2>/dev/null)"
            reason="$(echo "$entry_json" | node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).reason)" 2>/dev/null)"
            visual_glob="$(echo "$entry_json" | node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).visual_glob)" 2>/dev/null)"
            mismatch_action="$(echo "$entry_json" | node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).mismatch_action)" 2>/dev/null)"
            byte_start="$(echo "$entry_json" | node -e "process.stdout.write(String(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).byte_start))" 2>/dev/null)"
            byte_end="$(echo "$entry_json" | node -e "process.stdout.write(String(JSON.parse(require('fs').readFileSync('/dev/stdin','utf8')).byte_end))" 2>/dev/null)"
            src_globs_str="$(echo "$entry_json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
process.stdout.write(d.src_globs.join('\n'));
" 2>/dev/null)"

            # Find matching changed source files.
            # Reset (truncate, not just touch) the per-entry matching-sources file:
            # these temp files are keyed only by entry index within the shared
            # work dir, so without clearing it here a source matched for an
            # earlier doc's entry N would leak into this doc's entry N — including
            # empty-glob sections that never enter the matching loop below —
            # causing cross-doc mis-attribution on multi-file diffs.
            : > "$WORK_DIR/matching_sources_${idx}.txt"
            if [ -s "$WORK_DIR/changed_source.txt" ] && [ -n "$src_globs_str" ]; then
                while IFS= read -r sf; do
                    [ -z "$sf" ] && continue
                    while IFS= read -r glob; do
                        [ -z "$glob" ] && continue
                        if glob_match_js "$sf" "$glob"; then
                            echo "$sf" >> "$WORK_DIR/matching_sources_${idx}.txt"
                            echo "$sf" >> "$WORK_DIR/covered_sources.txt"
                            break
                        fi
                    done <<< "$src_globs_str"
                done < "$WORK_DIR/changed_source.txt"
                sort -u "$WORK_DIR/matching_sources_${idx}.txt" -o "$WORK_DIR/matching_sources_${idx}.txt"
            fi

            # Example staleness check (issue #919, spec §D3 step 6).
            # A verified example carries an `<!-- example-verified: <sha> <iso> -->`
            # marker (stamped by verify-examples.mjs). The example is STALE when its
            # marker SHA is older (a strict git ancestor) than the newest commit
            # touching this scope's src_globs — the documented command may no longer
            # reflect the code, so it must be re-verified. Unlike drift/visual_stale
            # this is a marker-vs-history check, INDEPENDENT of the current diff, so
            # it must run BEFORE the no-source-match skip below — every scope section
            # carrying a marker is checked, not only sections whose sources changed
            # in this diff. Same JSON shape family as visual_stale (an array of
            # objects with doc_file/heading/source_changed[]).
            if [ -f "$docfile" ] && [ -n "$src_globs_str" ]; then
                sec_body="$(head -c "${byte_end:-0}" "$docfile" 2>/dev/null | tail -c "+$(( ${byte_start:-0} + 1 ))" 2>/dev/null)" || sec_body=""
                marker_sha="$(printf '%s' "$sec_body" \
                    | grep -oE '<!-- example-verified: [^ ]+ [^ ]+ -->' \
                    | head -1 \
                    | sed -E 's/<!-- example-verified: ([^ ]+) .*/\1/')" || marker_sha=""
                if [ -n "$marker_sha" ]; then
                    # NUL-delimited file feed so paths with spaces / glob / shell
                    # metacharacters are passed argv-safe to git (never word-split
                    # or re-globbed). example_srcfiles holds newline paths (for the
                    # human-readable source_changed[] array); example_srcfiles_z
                    # holds the matching NUL stream consumed by `git log`/xargs.
                    : > "$WORK_DIR/example_srcfiles_${idx}.txt"
                    : > "$WORK_DIR/example_srcfiles_${idx}.z"
                    while IFS= read -r glob; do
                        [ -z "$glob" ] && continue
                        git -C "$AUTOSPEC_REPO_ROOT" ls-files -z -- "$glob" 2>/dev/null \
                            >> "$WORK_DIR/example_srcfiles_${idx}.z" || true
                    done <<< "$src_globs_str"
                    # Derive the newline list (dedup) for JSON from the NUL stream.
                    tr '\0' '\n' < "$WORK_DIR/example_srcfiles_${idx}.z" \
                        | grep -v '^$' | sort -u > "$WORK_DIR/example_srcfiles_${idx}.txt" 2>/dev/null || true
                    newest_commit=""
                    if [ -s "$WORK_DIR/example_srcfiles_${idx}.z" ]; then
                        newest_commit="$(xargs -0 git -C "$AUTOSPEC_REPO_ROOT" log -1 --format='%H' -- \
                            < "$WORK_DIR/example_srcfiles_${idx}.z" 2>/dev/null)" || newest_commit=""
                    fi
                    if [ -n "$newest_commit" ] && [ "$newest_commit" != "$marker_sha" ]; then
                        # Stale only when the marker is a strict ANCESTOR of the
                        # newest src commit (marker predates the latest src change).
                        if git -C "$AUTOSPEC_REPO_ROOT" merge-base --is-ancestor "$marker_sha" "$newest_commit" 2>/dev/null; then
                            src_changed=""
                            while IFS= read -r sf; do
                                [ -z "$sf" ] && continue
                                src_changed="${src_changed:+$src_changed,}\"$sf\""
                            done < "$WORK_DIR/example_srcfiles_${idx}.txt"
                            h_esc="$(echo "$heading" | sed 's/"/\\"/g')"
                            printf '{"doc_file":"%s","heading":"%s","marker_sha":"%s","newest_src_commit":"%s","source_changed":[%s]}\n' \
                                "$rel_docfile" "$h_esc" "$marker_sha" "$newest_commit" "$src_changed" \
                                >> "$WORK_DIR/example_stale_entries.txt"
                        fi
                    fi
                fi
            fi

            # Skip if no source matches
            if [ ! -s "$WORK_DIR/matching_sources_${idx}.txt" ]; then
                idx=$((idx + 1))
                continue
            fi

            # Check if doc section was updated in this diff.
            # Strategy: if the doc file was changed in this diff AND the section's
            # byte range overlaps with any changed line, mark as updated.
            # For robustness we use a generous check: if the doc file is in the
            # changed list, compute section line range and check against changed lines.
            # If byte_start/byte_end are unavailable, fall back to doc-file-level check.
            section_updated=false
            if grep -qF "$rel_docfile" "$WORK_DIR/changed_docs.txt" 2>/dev/null; then
                safe_name="$(echo "$rel_docfile" | tr '/' '_')"
                changed_lines_file="$WORK_DIR/changedlines_${safe_name}.txt"
                if [ -f "$changed_lines_file" ] && [ -s "$changed_lines_file" ] && [ -f "$docfile" ]; then
                    # Compute section line range from byte offsets
                    # byte_start = offset of first byte AFTER the scope comment
                    # We add 1 to be inclusive of the line following the comment
                    sec_start_line="$(head -c "$byte_start" "$docfile" 2>/dev/null | wc -l | tr -d ' \t')" || sec_start_line=0
                    sec_end_line="$(head -c "$byte_end" "$docfile" 2>/dev/null | wc -l | tr -d ' \t')" || sec_end_line=9999
                    # Check if any changed line is in the section (±2 lines tolerance)
                    tol_start=$(( ${sec_start_line:-0} - 2 ))
                    tol_end=$(( ${sec_end_line:-9999} + 2 ))
                    [ "$tol_start" -lt 0 ] && tol_start=0
                    while IFS= read -r lnum; do
                        [ -z "$lnum" ] && continue
                        if [ "$lnum" -ge "$tol_start" ] 2>/dev/null && \
                           [ "$lnum" -le "$tol_end" ] 2>/dev/null; then
                            section_updated=true
                            break
                        fi
                    done < "$changed_lines_file"
                elif grep -qF "$rel_docfile" "$WORK_DIR/changed_docs.txt" 2>/dev/null; then
                    # Fallback: doc file was touched but we can't compute line ranges
                    # Treat as updated (avoids false positives)
                    section_updated=true
                fi
            fi

            if ! $section_updated; then
                # Build matching_sources JSON array
                src_arr="$(while IFS= read -r sf; do
                    printf '"%s",' "$sf"
                done < "$WORK_DIR/matching_sources_${idx}.txt" | sed 's/,$//')"
                # Escape heading and reason for JSON
                h_esc="$(echo "$heading" | sed 's/"/\\"/g')"
                r_esc="$(echo "$reason" | sed 's/"/\\"/g')"
                # Bucket by mismatch_action: warn → drift_warn, hard_fail → drift
                if [ "${mismatch_action:-hard_fail}" = "warn" ]; then
                    printf '{"doc_file":"%s","heading":"%s","matching_source_files":[%s],"reason":"%s"}\n' \
                        "$rel_docfile" "$h_esc" "$src_arr" "$r_esc" >> "$WORK_DIR/drift_warn_entries.txt"
                else
                    printf '{"doc_file":"%s","heading":"%s","matching_source_files":[%s],"reason":"%s"}\n' \
                        "$rel_docfile" "$h_esc" "$src_arr" "$r_esc" >> "$WORK_DIR/drift_entries.txt"
                fi
            fi

            # Visual staleness check
            if [ -n "$visual_glob" ] && [ -s "$WORK_DIR/matching_sources_${idx}.txt" ]; then
                find "$AUTOSPEC_REPO_ROOT" -name "$(basename "$visual_glob")" 2>/dev/null | while IFS= read -r screenshot; do
                    rel_screenshot="${screenshot#${AUTOSPEC_REPO_ROOT}/}"
                    screenshot_mtime=0
                    if [ -f "$screenshot" ]; then
                        screenshot_mtime="$(stat -c '%Y' "$screenshot" 2>/dev/null || stat -f '%m' "$screenshot" 2>/dev/null || echo 0)"
                    fi
                    stale_sources=""
                    while IFS= read -r sf; do
                        [ -z "$sf" ] && continue
                        full_sf="${AUTOSPEC_REPO_ROOT}/${sf}"
                        if [ -f "$full_sf" ]; then
                            src_mtime="$(stat -c '%Y' "$full_sf" 2>/dev/null || stat -f '%m' "$full_sf" 2>/dev/null || echo 0)"
                            if [ "$src_mtime" -gt "$screenshot_mtime" ] 2>/dev/null; then
                                stale_sources="${stale_sources:+$stale_sources,}\"$sf\""
                            fi
                        fi
                    done < "$WORK_DIR/matching_sources_${idx}.txt"
                    if [ -n "$stale_sources" ]; then
                        h_esc="$(echo "$heading" | sed 's/"/\\"/g')"
                        printf '{"doc_file":"%s","heading":"%s","screenshot":"%s","source_changed":[%s]}\n' \
                            "$rel_docfile" "$h_esc" "$rel_screenshot" "$stale_sources" >> "$WORK_DIR/visual_stale_entries.txt"
                    fi
                done || true
            fi

            idx=$((idx + 1))
        done
    done < "$WORK_DIR/doc_files.txt"
fi

# ── Missing scope: source files not covered by any scope ─────────────────────

touch "$WORK_DIR/missing_scope_entries.txt"
sort -u "$WORK_DIR/covered_sources.txt" -o "$WORK_DIR/covered_sources.txt" 2>/dev/null || true

if [ -s "$WORK_DIR/changed_source.txt" ]; then
    while IFS= read -r sf; do
        [ -z "$sf" ] && continue
        if ! grep -qF "$sf" "$WORK_DIR/covered_sources.txt" 2>/dev/null; then
            printf '{"source_file":"%s","suggestion":"no docs section declares scope for this path"}\n' \
                "$sf" >> "$WORK_DIR/missing_scope_entries.txt"
        fi
    done < "$WORK_DIR/changed_source.txt"
fi

# ── Build JSON arrays ─────────────────────────────────────────────────────────

build_json_array() {
    local file="$1"
    if [ -s "$file" ]; then
        echo -n "["
        first=true
        while IFS= read -r line; do
            [ -z "$line" ] && continue
            $first || echo -n ","
            echo -n "$line"
            first=false
        done < "$file"
        echo -n "]"
    else
        echo -n "[]"
    fi
}

drift_arr="$(build_json_array "$WORK_DIR/drift_entries.txt")"
drift_warn_arr="$(build_json_array "$WORK_DIR/drift_warn_entries.txt")"
missing_arr="$(build_json_array "$WORK_DIR/missing_scope_entries.txt")"
vstale_arr="$(build_json_array "$WORK_DIR/visual_stale_entries.txt")"
example_stale_arr="$(build_json_array "$WORK_DIR/example_stale_entries.txt")"

# ── Apply docs: skip ──────────────────────────────────────────────────────────

if $DOCS_SKIP || $NO_SCOPE_REPO; then
    passed=true
    exit_code=0
    skipped_val=true
    drift_arr="[]"
    drift_warn_arr="[]"
    missing_arr="[]"
    # example_stale (like visual_stale) is a non-blocking signal and is NOT
    # zeroed under docs: skip / no-scope — it informs the regenerate self-heal.
else
    drift_count=0
    missing_count=0
    [ -s "$WORK_DIR/drift_entries.txt" ] && drift_count="$(wc -l < "$WORK_DIR/drift_entries.txt" | tr -d ' \n')" || drift_count=0
    [ -s "$WORK_DIR/missing_scope_entries.txt" ] && missing_count="$(wc -l < "$WORK_DIR/missing_scope_entries.txt" | tr -d ' \n')" || missing_count=0

    # Exit 1 only when hard_fail drift entries are present; warn-only drift does not block
    if [ "${drift_count:-0}" -gt 0 ]; then
        passed=false
        exit_code=1
    elif [ "${missing_count:-0}" -gt 0 ]; then
        passed=false
        exit_code=2
    else
        passed=true
        exit_code=0
    fi
    skipped_val=false
fi

# ── Emit JSON ─────────────────────────────────────────────────────────────────

cat <<JSON
{
  "passed": ${passed},
  "drift": ${drift_arr},
  "drift_warn": ${drift_warn_arr},
  "missing_scope": ${missing_arr},
  "visual_stale": ${vstale_arr},
  "example_stale": ${example_stale_arr},
  "ai_review_stale": [],
  "skipped": ${skipped_val}
}
JSON

exit "$exit_code"
