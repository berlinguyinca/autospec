#!/usr/bin/env bash
# autodetect.sh — probe a target repo for autospec-test contract defaults.
#
# Usage: autodetect <repo_root>
#   Outputs per-field defaults as JSON to stdout.
#   Exit 0 on success, exit 1 on fatal error.
#
# Probes performed:
#   - package.json scripts (test, test:e2e, e2e, start:e2e, dev)
#   - playwright.config.* glob
#   - env vars: E2E_BASE_URL, PLAYWRIGHT_BASE_URL, BASE_URL
#   - language markers: go.mod, Cargo.toml, pyproject.toml, pom.xml, build.gradle
#
# stdout: JSON with keys for any detected values; missing keys = not detected.
# stdin/stdout JSON convention: this script outputs JSON to stdout only.

set -eu

autodetect() {
    local repo_root="${1:-.}"

    if [ ! -d "$repo_root" ]; then
        printf '{"error":"repo_root not found: %s"}\n' "$repo_root" >&2
        exit 1
    fi

    # ── Language detection ─────────────────────────────────────────────────────
    local language=""
    local coverage_collector=""
    local unit_test_cmd=""

    if [ -f "$repo_root/go.mod" ]; then
        language="go"
        coverage_collector="go-cover"
        unit_test_cmd="go test ./... -cover"
    elif [ -f "$repo_root/Cargo.toml" ]; then
        language="rust"
        coverage_collector="cargo-llvm-cov"
        unit_test_cmd="cargo test"
    elif [ -f "$repo_root/pyproject.toml" ] || [ -f "$repo_root/setup.py" ] || [ -f "$repo_root/setup.cfg" ]; then
        language="python"
        coverage_collector="coverage-py"
        unit_test_cmd="pytest"
    elif [ -f "$repo_root/pom.xml" ]; then
        language="java"
        coverage_collector="jacoco"
        unit_test_cmd="mvn test"
    elif [ -f "$repo_root/build.gradle" ] || [ -f "$repo_root/build.gradle.kts" ]; then
        language="java"
        coverage_collector="jacoco"
        unit_test_cmd="./gradlew test"
    elif [ -f "$repo_root/package.json" ]; then
        language="node"
    fi

    # ── Node: probe package.json scripts ──────────────────────────────────────
    local e2e_start_cmd=""
    local playwright_cmd=""
    local e2e_test_cmd=""

    if [ -f "$repo_root/package.json" ] && command -v jq >/dev/null 2>&1; then
        # Unit test cmd from package.json
        if [ -z "$unit_test_cmd" ]; then
            local pkg_test
            pkg_test=$(jq -r '.scripts.test // empty' "$repo_root/package.json" 2>/dev/null || true)
            if [ -n "$pkg_test" ]; then
                unit_test_cmd="${pkg_test} --coverage"
            fi
        fi

        # Coverage collector for node
        if [ -z "$coverage_collector" ]; then
            # Prefer c8 if listed, else istanbul
            if jq -e '.devDependencies.c8 // .dependencies.c8' "$repo_root/package.json" >/dev/null 2>&1; then
                coverage_collector="c8"
            else
                coverage_collector="istanbul"
            fi
        fi

        # E2E start cmd
        local start_e2e
        start_e2e=$(jq -r '(.scripts["start:e2e"] // .scripts.dev // empty)' "$repo_root/package.json" 2>/dev/null || true)
        if [ -n "$start_e2e" ]; then
            e2e_start_cmd="$start_e2e"
        fi

        # E2E test cmd
        local e2e_pkg
        e2e_pkg=$(jq -r '(.scripts["test:e2e"] // .scripts.e2e // empty)' "$repo_root/package.json" 2>/dev/null || true)
        if [ -n "$e2e_pkg" ]; then
            e2e_test_cmd="$e2e_pkg"
            playwright_cmd="$e2e_pkg"
        fi
    fi

    # ── Playwright config detection ────────────────────────────────────────────
    local playwright_config=""
    for ext in ts js mjs cjs; do
        local cfg_path="$repo_root/playwright.config.$ext"
        if [ -f "$cfg_path" ]; then
            playwright_config="playwright.config.$ext"
            if [ -z "$playwright_cmd" ]; then
                playwright_cmd="npx playwright test"
            fi
            break
        fi
    done

    # ── Clone URL env detection ────────────────────────────────────────────────
    local clone_url_env=""
    if [ -n "${E2E_BASE_URL:-}" ]; then
        clone_url_env="E2E_BASE_URL"
    elif [ -n "${PLAYWRIGHT_BASE_URL:-}" ]; then
        clone_url_env="PLAYWRIGHT_BASE_URL"
    elif [ -n "${BASE_URL:-}" ]; then
        clone_url_env="BASE_URL"
    else
        # Check if any of these vars are referenced in package.json or playwright config
        if [ -f "$repo_root/package.json" ] && grep -q "E2E_BASE_URL" "$repo_root/package.json" 2>/dev/null; then
            clone_url_env="E2E_BASE_URL"
        elif [ -f "$repo_root/package.json" ] && grep -q "PLAYWRIGHT_BASE_URL" "$repo_root/package.json" 2>/dev/null; then
            clone_url_env="PLAYWRIGHT_BASE_URL"
        else
            clone_url_env="E2E_BASE_URL"
        fi
    fi

    # ── Emit JSON ──────────────────────────────────────────────────────────────
    # Build JSON output carefully; strip null/empty values at all levels
    # so downstream merge does not inject null into the schema-validated contract.
    jq -n \
        --arg language "$language" \
        --arg coverage_collector "$coverage_collector" \
        --arg unit_test_cmd "$unit_test_cmd" \
        --arg e2e_start_cmd "$e2e_start_cmd" \
        --arg playwright_cmd "$playwright_cmd" \
        --arg playwright_config "$playwright_config" \
        --arg clone_url_env "$clone_url_env" \
        '
        def nonempty(v): if v != "" and v != null then v else null end;
        def compact: with_entries(select(.value != null and .value != {}));

        {
            unit: ({
                test_cmd: nonempty($unit_test_cmd),
                coverage_collector: nonempty($coverage_collector)
            } | compact),
            e2e: ({
                clone_url_env: nonempty($clone_url_env),
                start_cmd: nonempty($e2e_start_cmd),
                playwright_cmd: nonempty($playwright_cmd),
                playwright_config: nonempty($playwright_config)
            } | compact)
        } | compact
        '
}

autodetect "${1:-}"
