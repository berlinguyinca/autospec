#!/usr/bin/env bash
# scripts/lib/lint-unwired-pub.sh — UNWIRED_PUB_ITEM detector (issue #4346).
#
# Three consecutive conversions (#4312, #4295, #315) each shipped a well-tested
# capability that nothing calls: the gate sees "compiles + tests pass" and stops,
# because an unused pub fn with thorough tests looks exactly like a well-tested
# one in use. Invariant 2 of #4346: a patch introducing a pub item must show
# its caller, or say why it has none yet — the two must not be
# indistinguishable in a diff. This detector is the cheap mechanical half: it
# flags an added `pub fn` / `pub struct` in a Rust diff with zero references
# outside its own test module. The PR then says which it is — forgotten wiring
# or deliberate staging — via `Guardian: skip-UNWIRED_PUB_ITEM # <reason>` or
# the inline `# linter:allow-UNWIRED_PUB_ITEM <reason>` hatch; both require a
# justification, so "staging" is a declaration, not a silent default.
#
# Pre-commit / staged mode only: the detector cross-references the working
# tree, and only there does the tree contain the change itself. In PR mode the
# tree sits at the base commit, where a caller added by the same PR does not
# exist yet, so the rule would flag every newly wired item.
#
# Sourced, not executed. Relies on the caller's emit_capped / emit_info /
# is_line_allowed / get_diff_files / get_added_lines_with_lineno / is_test_file
# helpers. bash 3.2+. No `set -e` here: the caller owns shell options.

# ── rg dependency (fail-open, never silent) ───────────────────────────────────
_upi_rg_available() {
    if command -v rg >/dev/null 2>&1; then
        return 0
    fi
    if [ "${_UPI_RG_NOTICE_EMITTED:-0}" != "1" ]; then
        _UPI_RG_NOTICE_EMITTED=1
        emit_info UNWIRED_PUB_ITEM "-" "-" \
            'ripgrep (rg) not found on PATH; UNWIRED_PUB_ITEM is inert this run. If `rg` works in your shell it may be a shell function, which child processes do not inherit — install the ripgrep binary.'
    fi
    return 1
}

# _upi_test_module_lines FILE — print the line numbers inside #[cfg(test)]
# modules. A test module is a `mod X {` line whose immediately preceding
# attribute is #[cfg(test)] (on the previous line or the same line), closed by
# the first column-0 `}`. Deliberately textual: a reference inside the test
# module is the exact pattern this rule exists to exclude.
_upi_test_module_lines() {
    awk '
        /#\[[[:space:]]*cfg[[:space:]]*\([[:space:]]*test[[:space:]]*\)/ &&
            /mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/ {
            in_test = 1
            print NR
            next
        }
        in_test {
            print NR
            if ($0 ~ /^\}/) in_test = 0
            next
        }
        /#\[[[:space:]]*cfg[[:space:]]*\([[:space:]]*test[[:space:]]*\)/ { pending = 1; next }
        pending {
            if ($0 ~ /^[[:space:]]*mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/) in_test = 1
            pending = 0
        }
    ' "$1" 2>/dev/null || true
}

# detect_unwired_pub_item — RULE_ID UNWIRED_PUB_ITEM (#4346).
#
# One finding per added `pub fn` / `pub struct` in a non-test .rs diff file
# with no word-boundary reference in any other .rs file in the tree AND no
# reference in the defining file outside its #[cfg(test)] module. A reference
# from any other file counts as wiring, including a `pub use` re-export, which
# is the wiring for a library's public API. A bare `mod NAME;` / `mod NAME {`
# declaration is NOT a call: #4312's capability sat behind exactly that.
detect_unwired_pub_item() {
    [ "$STAGED" -eq 1 ] || return 0
    _upi_rg_available || return 0

    local _upi_tmp
    _upi_tmp="$(mktemp -t lint-unwired-pub.XXXXXX)"

    # Collect "FILE:LINENO:NAME" for each added pub item.
    local _upi_file _upi_lno _upi_content _upi_name
    while IFS= read -r _upi_file; do
        [ -z "$_upi_file" ] && continue
        case "$_upi_file" in *.rs) ;; *) continue ;; esac
        is_test_file "$_upi_file" && continue
        while IFS=: read -r _upi_lno _upi_content; do
            _upi_name="$(printf '%s' "$_upi_content" \
                | sed -nE 's/^[[:space:]]*pub[[:space:]]+(fn|struct)[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)([^A-Za-z0-9_]|$).*/\2/p')"
            [ -n "$_upi_name" ] || continue
            printf '%s\n' "$_upi_file:$_upi_lno:$_upi_name" >> "$_upi_tmp"
        done <<EOF
$(get_added_lines_with_lineno "$_upi_file")
EOF
    done <<EOF
$(get_diff_files)
EOF

    while IFS=: read -r _upi_file _upi_lno _upi_name; do
        [ -n "$_upi_name" ] || continue

        # 1) A reference in any other file is wiring (call site or re-export).
        #    rg exits 0 = matches, 1 = none, 2+ = tooling error.
        local _upi_refs _upi_rg_rc=0
        _upi_refs="$(rg -wF "$_upi_name" -n -g '*.rs' . 2>/dev/null)" || _upi_rg_rc=$?
        if [ "$_upi_rg_rc" -ge 2 ]; then
            if [ "${_UPI_COULD_NOT_RUN_EMITTED:-0}" != "1" ]; then
                _UPI_COULD_NOT_RUN_EMITTED=1
                emit_info UNWIRED_PUB_ITEM "$_upi_file" "$_upi_lno" \
                    "ripgrep exited ${_upi_rg_rc} while searching references; UNWIRED_PUB_ITEM skipped (fail-open, no finding)"
            fi
            continue
        fi
        # Drop the defining file (judged below with the test module removed)
        # and bare module declarations, which name the item without calling it.
        local _upi_external
        _upi_external="$(printf '%s\n' "$_upi_refs" \
            | grep -vF "./$_upi_file:" \
            | grep -vE ":[0-9]+:[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+${_upi_name}([[:space:]]*;|[[:space:]]*\{)" \
            || true)"
        [ -n "$_upi_external" ] && continue

        # 2) Same file: a reference outside the #[cfg(test)] module is wiring.
        #    The definition line itself is not a caller.
        if [ -f "$_upi_file" ]; then
            local _upi_testlines _upi_occ _upi_ref _upi_wired=0
            _upi_testlines="$(_upi_test_module_lines "$_upi_file")"
            _upi_occ="$(grep -nE "(^|[^A-Za-z0-9_])${_upi_name}([^A-Za-z0-9_]|$)" "$_upi_file" 2>/dev/null | cut -d: -f1 || true)"
            while IFS= read -r _upi_ref; do
                [ -n "$_upi_ref" ] || continue
                [ "$_upi_ref" = "$_upi_lno" ] && continue
                printf '%s\n' "$_upi_testlines" | grep -qxF "$_upi_ref" && continue
                _upi_wired=1
                break
            done <<EOF
$_upi_occ
EOF
            [ "$_upi_wired" -eq 1 ] && continue
        fi

        if ! is_line_allowed "UNWIRED_PUB_ITEM" "$_upi_file" "$_upi_lno"; then
            emit_capped "UNWIRED_PUB_ITEM" "$_upi_file" "$_upi_lno" \
                "'$_upi_name' added as pub with no reference outside its own test module — show the caller that wires it, or declare deliberate staging (Guardian: skip-UNWIRED_PUB_ITEM # <reason>) (#4346)"
        fi
    done < "$_upi_tmp"

    rm -f "$_upi_tmp"
}
