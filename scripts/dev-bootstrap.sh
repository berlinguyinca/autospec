#!/usr/bin/env bash
# scripts/dev-bootstrap.sh — install bats-core and verify required tools.
#
# Spec reference: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.5
#
# This script makes a fresh contributor checkout runnable: it detects the
# system package manager (brew / apt / npm), installs bats-core and the ajv
# JSON-schema CLI if missing, and verifies that gh, jq, python3, and ajv are on
# PATH. It exits non-zero with a helpful diagnostic on missing tools so CI / a
# pre-commit cannot proceed silently into a broken state.

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

# install_ajv — install the ajv JSON-schema CLI. Idempotent: a present ajv is
# left alone.
#
# ajv is not optional. `install.sh` lists it in AUTOSPEC_SYSTEM_TOOLS and
# fleet-config-lint.sh calls `need_cmd ajv` before validating
# autospec-fleet.yml against its draft-2020-12 schema, so every fleet script —
# and therefore the check_autospec_fleet_enabled_false validate gate — is dead
# without it. Bootstrapping it here rather than in a CI workflow keeps one
# install path shared by contributors and CI.
install_ajv() {
    if command -v ajv > /dev/null 2>&1; then
        info "ajv already installed: $(command -v ajv)"
        return 0
    fi
    ensure_tool="$repo_root/skills/autospec-shared/scripts/ensure-tool.sh"
    if [ -f "$ensure_tool" ]; then
        info "installing ajv via ensure-tool.sh ..."
        bash "$ensure_tool" ajv || true
        hash -r 2> /dev/null || true
    fi
    if command -v ajv > /dev/null 2>&1; then
        return 0
    fi
    # A system-owned npm prefix (the default on plain Debian/Ubuntu, where
    # `npm config get prefix` is /usr) rejects a user-level global install,
    # so retry once under sudo — the same escalation install_bats already
    # uses for apt-get. Both attempts are one-sided by design and must be
    # `if`/`then`, never `cmd || true` chained onto a test: under `set -e`
    # a failing left-hand test in `[ ... ] && action` aborts the script.
    if command -v npm > /dev/null 2>&1 \
        && [ "$(id -u)" != "0" ] \
        && command -v sudo > /dev/null 2>&1; then
        info "installing ajv via sudo npm (global) ..."
        sudo npm install -g ajv-cli || true
        hash -r 2> /dev/null || true
    fi
    if ! command -v ajv > /dev/null 2>&1; then
        warn "ajv install did not succeed; check_tools will report it"
    fi
}

# check_tools — verify gh, jq, python3, ajv are present. Missing any causes a
# non-zero exit with a clear message.
check_tools() {
    missing=""
    for tool in gh jq python3 ajv; do
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
    repo_root="$(cd "$(dirname "$0")/.." && pwd)"
    install_bats
    install_ajv
    check_tools
    info "all required tools present"
    info ""
    info "next: run the test suite with:"
    info "  bats tests/unit tests/smoke && autospec validate"
}

main "$@"
