#!/usr/bin/env bash
# scripts/dev-bootstrap.sh — install bats-core and verify required tools.
#
# Spec reference: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.5
#
# This script makes a fresh contributor checkout runnable: it detects the
# system package manager (brew / apt / npm), installs bats-core if missing,
# and verifies that gh, jq, and python3 are on PATH. It exits non-zero with
# a helpful diagnostic on missing tools so CI / a pre-commit cannot proceed
# silently into a broken state.

set -eu

info() {
    printf 'dev-bootstrap: %s\n' "$*"
}

warn() {
    printf 'dev-bootstrap: WARN — %s\n' "$*" >&2
}

fail() {
    printf 'dev-bootstrap: FAIL — %s\n' "$*" >&2
    exit 1
}

# detect_pkg_manager — echo the first available package manager we know how
# to drive, or "none" if nothing is available.
detect_pkg_manager() {
    if command -v brew >/dev/null 2>&1; then
        printf 'brew\n'
    elif command -v apt-get >/dev/null 2>&1; then
        printf 'apt\n'
    elif command -v npm >/dev/null 2>&1; then
        printf 'npm\n'
    else
        printf 'none\n'
    fi
}

# install_bats — install bats-core via the chosen package manager. Idempotent:
# if `bats` is already on PATH we no-op.
install_bats() {
    if command -v bats >/dev/null 2>&1; then
        info "bats already installed: $(bats --version 2>/dev/null || echo present)"
        return 0
    fi

    mgr="$(detect_pkg_manager)"
    case "$mgr" in
        brew)
            info "installing bats-core via brew ..."
            brew install bats-core
            ;;
        apt)
            info "installing bats via apt-get ..."
            sudo apt-get update -y
            sudo apt-get install -y bats
            ;;
        npm)
            info "installing bats via npm (global) ..."
            npm install -g bats
            ;;
        none)
            fail "no supported package manager found (brew/apt/npm); install bats-core manually from https://github.com/bats-core/bats-core"
            ;;
    esac
}

# check_tools — verify gh, jq, python3 are present. Missing any causes a
# non-zero exit with a clear message.
check_tools() {
    missing=""
    for tool in gh jq python3; do
        if command -v "$tool" >/dev/null 2>&1; then
            ver="$("$tool" --version 2>/dev/null | head -1 || true)"
            info "$tool: ${ver:-present}"
        else
            warn "$tool: MISSING"
            missing="$missing $tool"
        fi
    done
    if [ -n "$missing" ]; then
        fail "required tools missing:$missing — install via your package manager and re-run"
    fi
}

main() {
    info "starting dev-bootstrap"
    install_bats
    check_tools
    info "all required tools present"
    info ""
    info "next: run the test suite with:"
    info "  bats tests/unit tests/smoke && autospec validate"
}

main "$@"
