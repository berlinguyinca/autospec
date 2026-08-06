#!/usr/bin/env bash
# scripts/lib/lint-reuse-lens.sh — the ripgrep-backed reuse detectors (issue #1439).
#
# Extracted from lint-implementation.sh, which the size ratchet correctly refuses
# to let grow. The boundary is the DEPENDENCY, not the feature: these are exactly
# the detectors that shell out to ripgrep, so their shared fail-open guard lives
# with them. NEW_DEP_UNJUSTIFIED stays in the parent script — it needs no rg.
#
# Sourced, not executed. Relies on the caller's emit_capped / emit_info /
# is_line_allowed / get_diff_files / get_added_lines_with_lineno helpers.
#
# bash 3.2+. No `set -e` here: the caller owns shell options.

# ── reuse-lens rg dependency ──────────────────────────────────────────────────
# Both detectors below shell out to ripgrep and fail OPEN when it is absent. That
# is correct — a missing search tool must never block a commit — but failing open
# SILENTLY is the defect this guard removes: a repo whose reuse lens detects
# nothing looks exactly like a repo with no reuse problems, so build-vs-buy BLOCKs
# stop being raised and the precision ledger records nothing rather than a miss.
#
# Why it hides so well: `rg` is often a shell FUNCTION rather than a binary (agent
# harnesses inject one proxying to a bundled copy), and shell functions are not
# exported to child processes. `command -v rg` therefore succeeds at the
# operator's prompt and fails inside every script autospec runs.
#
# Emitted once per run, as INFO so it never blocks.
_reuse_lens_rg_available() {
    if command -v rg >/dev/null 2>&1; then
        return 0
    fi
    if [ "${_REUSE_LENS_RG_NOTICE_EMITTED:-0}" != "1" ]; then
        _REUSE_LENS_RG_NOTICE_EMITTED=1
        emit_info REUSE_LENS_DISABLED "-" "-" \
            'ripgrep (rg) not found on PATH; REINVENT_REPO_UTIL and NEW_ABSTRACTION_SINGLE_CALLER are inert. If `rg` works in your shell it may be a shell function, which child processes do not inherit — install the ripgrep binary.'
    fi
    return 1
}

# ── §3.x REINVENT_REPO_UTIL detector ─────────────────────────────────────────
# Net-new function definition duplicating an existing helper found by rg across
# scripts/. Heuristic: new name() def where rg --fixed-strings finds it in
# another .sh/.bash file under scripts/. Fail-open: rg absent or error → silent.

detect_reinvent_repo_util() {
    if ! _reuse_lens_rg_available; then
        return 0
    fi

    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        case "$diff_file" in
            *.sh|*.bash) ;;
            *) continue ;;
        esac

        while IFS=: read -r lineno content; do
            # Match bash-style function definitions: name() { or function name() {
            if printf '%s' "$content" | grep -qE \
                '^[[:space:]]*(function[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(\)'; then
                # Extract function name, skipping the "function" keyword
                local _rru_name
                _rru_name="$(printf '%s' "$content" \
                    | sed 's/^[[:space:]]*//' \
                    | sed 's/^function[[:space:]]*//' \
                    | grep -oE '^[A-Za-z_][A-Za-z0-9_]*')"
                [ -z "$_rru_name" ] && continue
                # Skip ubiquitous generic names that legitimately recur
                case "$_rru_name" in
                    main|setup|teardown|run|help|usage|init|cleanup|die|err|warn|log) continue ;;
                esac
                # Search for existing definition in scripts/; fail-open on rg error
                local _rru_matches=""
                _rru_matches="$(rg --fixed-strings "${_rru_name}()" \
                    --glob '*.sh' --glob '*.bash' -l scripts/ 2>/dev/null || true)"
                [ -z "$_rru_matches" ] && continue
                # Exclude the file being added itself
                local _rru_others=""
                _rru_others="$(printf '%s\n' "$_rru_matches" \
                    | grep -vF "$diff_file" || true)"
                [ -z "$_rru_others" ] && continue
                if ! is_line_allowed "REINVENT_REPO_UTIL" "$diff_file" "$lineno"; then
                    local _rru_first
                    _rru_first="$(printf '%s\n' "$_rru_others" | head -1)"
                    emit_capped "REINVENT_REPO_UTIL" "$diff_file" "$lineno" \
                        "function '${_rru_name}' already defined in ${_rru_first}"
                fi
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.x NEW_ABSTRACTION_SINGLE_CALLER detector ───────────────────────────────
# Net-new file matching *manager*|*factory*|*adapter*|*wrapper*|*base*|*abstract*
# with ≤1 external call site found by rg in the tree. Fail-open: rg error → silent.

detect_new_abstraction_single_caller() {
    if ! _reuse_lens_rg_available; then
        return 0
    fi

    local _nasc_tmp
    _nasc_tmp="$(mktemp -t lint-nasc.XXXXXX)"

    # Collect all net-new files from the diff (those with "new file mode" header)
    awk '
        /^diff --git / {
            f = $0; sub(/^diff --git a\/[^ ]* b\//, "", f); is_new = 0
        }
        /^new file mode/ { is_new = 1 }
        is_new && /^@@ / { print f; is_new = 0 }
    ' "$TMP_DIFF" | sort -u > "$_nasc_tmp"

    while IFS= read -r _nasc_file; do
        [ -z "$_nasc_file" ] && continue
        local _nasc_bname _nasc_stem _nasc_lower
        _nasc_bname="$(basename "$_nasc_file")"
        _nasc_stem="$(printf '%s' "$_nasc_bname" | sed 's/\.[^.]*$//')"
        _nasc_lower="$(printf '%s' "$_nasc_stem" | tr '[:upper:]' '[:lower:]')"
        case "$_nasc_lower" in
            *manager*|*factory*|*adapter*|*wrapper*|*base*|*abstract*) ;;
            *) continue ;;
        esac
        # Check linter:allow on the diff file (line "-" means no specific line)
        # For new files with no line reference, check the file if it exists
        # Count external callers (files referencing this stem, excluding itself).
        # rg exits 0=matches found, 1=no matches, 2+=error.
        # Treat exit ≥2 as tooling error → fail-open (skip, no finding).
        local _nasc_rg_out _nasc_rg_status=0
        _nasc_rg_out="$(mktemp -t lint-nasc-rg.XXXXXX)"
        rg --fixed-strings "$_nasc_stem" -l . > "$_nasc_rg_out" 2>/dev/null \
            || _nasc_rg_status=$?
        if [ "$_nasc_rg_status" -ge 2 ]; then
            rm -f "$_nasc_rg_out"
            continue
        fi
        local _nasc_count=0
        _nasc_count="$(sed 's|^\./||' "$_nasc_rg_out" \
            | grep -vF "$_nasc_file" \
            | wc -l | tr -d ' ')"
        rm -f "$_nasc_rg_out"
        if [ "${_nasc_count:-0}" -le 1 ]; then
            emit_capped "NEW_ABSTRACTION_SINGLE_CALLER" "$_nasc_file" "-" \
                "new abstraction '${_nasc_stem}' has ${_nasc_count} external caller(s) — consider inlining if single-use"
        fi
    done < "$_nasc_tmp"

    rm -f "$_nasc_tmp"
}
