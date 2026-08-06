#!/usr/bin/env bats
# tests/lint/test_reuse_triage.bats — reuse-lens RULE_ID detectors (issue #1439).
#
# Tests REINVENT_REPO_UTIL, NEW_DEP_UNJUSTIFIED, and NEW_ABSTRACTION_SINGLE_CALLER
# using real git repos and rg (no mocks, per AGENTS.md).

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
    LINT="${REPO_ROOT}/scripts/lint-implementation.sh"

    # The reuse-triage detectors are part of the reuse lens and only run when
    # the lens is armed (issue #1439; spec flag-OFF inertness AC). Arm it for the
    # positive/negative detector fixtures; the dedicated inertness test below
    # unsets it explicitly.
    export AUTOSPEC_REUSE_LENS=1

    # Create an isolated temp git repo for each test
    FAKE_REPO="$(mktemp -d -t reuse-triage-test.XXXXXX)"
    cd "$FAKE_REPO"
    git init -q
    git config user.email "test@autospec.test"
    git config user.name "Autospec Test"
    mkdir -p scripts/lib

    # Commit a baseline so HEAD exists (needed for git diff --cached)
    printf '# base\n' > README.md
    git add README.md
    git commit -q -m "base"
}

teardown() {
    # Return to a safe dir before removing the temp repo
    cd /tmp
    rm -rf "$FAKE_REPO"
}

# These detectors shell out to ripgrep. Without an rg BINARY on PATH they fail
# open by design, so a positive-detection test cannot pass and a "detector stays
# silent" test would pass for the WRONG reason — it would be measuring the
# fail-open, not the negative case. Skip explicitly rather than reporting a red
# that reads like a detector regression (issue #2962).
#
# Note `command -v rg` is the right check and a shell function is NOT enough: some
# agent harnesses inject an `rg` function, which child processes never inherit.
require_rg_binary() {
    if ! command -v rg >/dev/null 2>&1; then
        skip "ripgrep binary not on PATH; rg-backed detectors fail open (see #2962)"
    fi
}

# ─── REINVENT_REPO_UTIL ───────────────────────────────────────────────────────

@test "REINVENT_REPO_UTIL: emits finding when new function duplicates existing helper" {
    require_rg_binary
    # Commit an existing utility with parse_config()
    cat > scripts/lib/config-utils.sh <<'SH'
#!/usr/bin/env bash
parse_config() { grep -E '^[A-Z_]+='; }
SH
    git add scripts/lib/config-utils.sh
    git commit -q -m "add config-utils"

    # Stage a new script that re-implements parse_config()
    cat > scripts/new-loader.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented version"; }
SH
    git add scripts/new-loader.sh

    run bash "$LINT" --pre-commit --staged
    local count
    count="$(printf '%s\n' "$output" | grep -c 'REINVENT_REPO_UTIL' || true)"
    [ "$count" -ge 1 ]
}

@test "REINVENT_REPO_UTIL: suppressed by linter:allow-REINVENT_REPO_UTIL with reason" {
    # Commit existing helper
    cat > scripts/lib/config-utils.sh <<'SH'
#!/usr/bin/env bash
parse_config() { grep -E '^[A-Z_]+='; }
SH
    git add scripts/lib/config-utils.sh
    git commit -q -m "add config-utils"

    # Stage new file with allow annotation on the preceding line
    cat > scripts/new-loader.sh <<'SH'
#!/usr/bin/env bash
# linter:allow-REINVENT_REPO_UTIL local variant needed for different separator
parse_config() { echo "allowed variant"; }
SH
    git add scripts/new-loader.sh

    run bash "$LINT" --pre-commit --staged
    # Must not emit a finding (INFO is ok, but not a blocking REINVENT_REPO_UTIL line)
    local findings
    findings="$(printf '%s\n' "$output" | grep '^REINVENT_REPO_UTIL:' || true)"
    [ -z "$findings" ]
}

@test "REINVENT_REPO_UTIL: silent when no duplicate exists in scripts/" {
    # Stage a new script with a unique function name (no existing helper)
    cat > scripts/unique-loader.sh <<'SH'
#!/usr/bin/env bash
load_unique_dataset_v7() { echo "no conflict"; }
SH
    git add scripts/unique-loader.sh

    run bash "$LINT" --pre-commit --staged
    local findings
    findings="$(printf '%s\n' "$output" | grep '^REINVENT_REPO_UTIL:' || true)"
    [ -z "$findings" ]
}

# ─── NEW_DEP_UNJUSTIFIED ──────────────────────────────────────────────────────

@test "NEW_DEP_UNJUSTIFIED: emits finding for requirements.txt dep add without why:" {
    # Stage a requirements.txt with a new dep and no why: comment
    cat > requirements.txt <<'TXT'
requests==2.28.2
TXT
    git add requirements.txt

    run bash "$LINT" --pre-commit --staged
    local count
    count="$(printf '%s\n' "$output" | grep -c 'NEW_DEP_UNJUSTIFIED' || true)"
    [ "$count" -ge 1 ]
}

@test "NEW_DEP_UNJUSTIFIED: silent when why: comment is present in the same hunk" {
    # Stage a requirements.txt with a why: comment on an adjacent line
    cat > requirements.txt <<'TXT'
# why: requests is needed for HTTP calls to the reporting API
requests==2.28.2
TXT
    git add requirements.txt

    run bash "$LINT" --pre-commit --staged
    local findings
    findings="$(printf '%s\n' "$output" | grep '^NEW_DEP_UNJUSTIFIED:' || true)"
    [ -z "$findings" ]
}

@test "NEW_DEP_UNJUSTIFIED: silent for non-manifest files" {
    # Stage a plain shell script — not a manifest
    cat > scripts/deploy.sh <<'SH'
#!/usr/bin/env bash
echo "deploy"
SH
    git add scripts/deploy.sh

    run bash "$LINT" --pre-commit --staged
    local findings
    findings="$(printf '%s\n' "$output" | grep '^NEW_DEP_UNJUSTIFIED:' || true)"
    [ -z "$findings" ]
}

# ─── NEW_ABSTRACTION_SINGLE_CALLER ───────────────────────────────────────────

@test "NEW_ABSTRACTION_SINGLE_CALLER: emits finding for new *-manager file with no callers" {
    require_rg_binary
    # Stage a new manager file with zero external callers
    cat > scripts/session-manager.sh <<'SH'
#!/usr/bin/env bash
manage_session() { echo "session"; }
SH
    git add scripts/session-manager.sh

    run bash "$LINT" --pre-commit --staged
    local count
    count="$(printf '%s\n' "$output" | grep -c 'NEW_ABSTRACTION_SINGLE_CALLER' || true)"
    [ "$count" -ge 1 ]
}

@test "NEW_ABSTRACTION_SINGLE_CALLER: silent when abstraction has 2+ external callers" {
    require_rg_binary
    # Commit two caller files that reference the manager stem
    cat > scripts/cmd-a.sh <<'SH'
#!/usr/bin/env bash
# uses session-manager
bash scripts/session-manager.sh
SH
    cat > scripts/cmd-b.sh <<'SH'
#!/usr/bin/env bash
# also uses session-manager
bash scripts/session-manager.sh
SH
    git add scripts/cmd-a.sh scripts/cmd-b.sh
    git commit -q -m "add callers"

    # Stage the new manager file
    cat > scripts/session-manager.sh <<'SH'
#!/usr/bin/env bash
manage_session() { echo "session"; }
SH
    git add scripts/session-manager.sh

    run bash "$LINT" --pre-commit --staged
    local findings
    findings="$(printf '%s\n' "$output" | grep '^NEW_ABSTRACTION_SINGLE_CALLER:' || true)"
    [ -z "$findings" ]
}

# ─── Clean diff (all three detectors silent) ──────────────────────────────────

@test "clean diff: no reuse findings on a plain implementation file" {
    # Stage a simple script with unique function names, no manifest, not an abstraction
    cat > scripts/reporter.sh <<'SH'
#!/usr/bin/env bash
generate_report_output_v2() { echo "report"; }
SH
    git add scripts/reporter.sh

    run bash "$LINT" --pre-commit --staged
    local reuse_findings
    reuse_findings="$(printf '%s\n' "$output" \
        | grep -E '^(REINVENT_REPO_UTIL|NEW_DEP_UNJUSTIFIED|NEW_ABSTRACTION_SINGLE_CALLER):' \
        || true)"
    [ -z "$reuse_findings" ]
}

# ─── Flag-OFF inertness (spec: lens inert unless AUTOSPEC_REUSE_LENS=1) ───────

@test "flag OFF: AUTOSPEC_REUSE_LENS unset → reuse detectors emit nothing" {
    # Same positive fixture as REINVENT_REPO_UTIL above — would fire when armed.
    cat > scripts/lib/config-utils.sh <<'SH'
#!/usr/bin/env bash
parse_config() { grep -E '^[A-Z_]+='; }
SH
    git add scripts/lib/config-utils.sh
    git commit -q -m "add config-utils"

    cat > scripts/new-loader.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented version"; }
SH
    git add scripts/new-loader.sh

    unset AUTOSPEC_REUSE_LENS
    run bash "$LINT" --pre-commit --staged
    local reuse_findings
    reuse_findings="$(printf '%s\n' "$output" \
        | grep -E '^(REINVENT_REPO_UTIL|NEW_DEP_UNJUSTIFIED|NEW_ABSTRACTION_SINGLE_CALLER):' \
        || true)"
    [ -z "$reuse_findings" ]
}

# ─── Fail-open: rg unavailable ───────────────────────────────────────────────

@test "rg unavailable: REINVENT_REPO_UTIL and ABSTRACTION detectors are silent (fail-open)" {
    # Commit an existing helper so REINVENT_REPO_UTIL would fire if rg worked
    cat > scripts/lib/config-utils.sh <<'SH'
#!/usr/bin/env bash
parse_config() { grep -E '^[A-Z_]+='; }
SH
    git add scripts/lib/config-utils.sh
    git commit -q -m "add config-utils"

    # Stage a file that re-implements parse_config() and is named *-manager
    cat > scripts/cache-manager.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented"; }
SH
    git add scripts/cache-manager.sh

    # Shadow rg with a script that exits 2 (tooling error, not "no matches").
    # rg uses exit 0=found, 1=no matches, 2+=error; detectors must be silent on ≥2.
    local fake_bin="$FAKE_REPO/fakebin"
    mkdir -p "$fake_bin"
    printf '#!/usr/bin/env bash\nexit 2\n' > "$fake_bin/rg"
    chmod +x "$fake_bin/rg"

    PATH="$fake_bin:$PATH" run bash "$LINT" --pre-commit --staged
    [ "$status" -eq 0 ]
    local rru_findings
    rru_findings="$(printf '%s\n' "$output" \
        | grep -E '^(REINVENT_REPO_UTIL|NEW_ABSTRACTION_SINGLE_CALLER):' || true)"
    [ -z "$rru_findings" ]
}

# ─── Fail-open must be LOUD when rg is absent entirely ───────────────────────
# Distinct from the test above, which shadows rg with a binary that ERRORS. Here
# rg does not exist at all, which is the case that silently disarmed the whole
# reuse lens on a real host (issue #2962).

@test "rg absent: the reuse lens announces itself as disabled instead of going quiet" {
    cat > scripts/lib/config-utils.sh <<'SH'
#!/usr/bin/env bash
parse_config() { grep -E '^[A-Z_]+='; }
SH
    git add scripts/lib/config-utils.sh
    git commit -q -m "add config-utils"

    cat > scripts/new-loader.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented version"; }
SH
    git add scripts/new-loader.sh

    # A PATH with no rg on it at all. Keep the essentials the lint script needs.
    local bare_bin="$FAKE_REPO/barebin"
    mkdir -p "$bare_bin"
    for tool in bash git grep sed awk cut sort uniq head tail wc mktemp rm cat printf tr find comm diff; do
        if command -v "$tool" >/dev/null 2>&1; then
            ln -sf "$(command -v "$tool")" "$bare_bin/$tool" 2>/dev/null
        fi
    done

    run env PATH="$bare_bin" AUTOSPEC_REUSE_LENS=1 bash "$LINT" --pre-commit --staged
    # Non-blocking: a missing search tool must never stop a commit.
    [ "$status" -eq 0 ]
    # ...but it must SAY so. A silent inert lens is indistinguishable from a clean repo.
    [[ "$output" == *"REUSE_LENS_DISABLED"* ]]
    [[ "$output" == *"ripgrep"* ]]
    # Names the detectors it disabled, so the operator knows what stopped running.
    [[ "$output" == *"REINVENT_REPO_UTIL"* ]]
    [[ "$output" == *"NEW_ABSTRACTION_SINGLE_CALLER"* ]]
}

@test "rg absent: the disabled notice is emitted once, not once per detector" {
    cat > scripts/cache-manager.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented"; }
SH
    git add scripts/cache-manager.sh

    local bare_bin="$FAKE_REPO/barebin2"
    mkdir -p "$bare_bin"
    for tool in bash git grep sed awk cut sort uniq head tail wc mktemp rm cat printf tr find comm diff; do
        if command -v "$tool" >/dev/null 2>&1; then
            ln -sf "$(command -v "$tool")" "$bare_bin/$tool" 2>/dev/null
        fi
    done

    run env PATH="$bare_bin" AUTOSPEC_REUSE_LENS=1 bash "$LINT" --pre-commit --staged
    [ "$status" -eq 0 ]
    local notices
    notices="$(printf '%s\n' "$output" | grep -c 'REUSE_LENS_DISABLED')"
    [ "$notices" -eq 1 ]
}

@test "rg absent with the lens disarmed stays completely silent" {
    # The notice belongs to the reuse lens, so it must not appear when the lens
    # is off — otherwise every unrelated commit on an rg-less host gets noise.
    cat > scripts/cache-manager.sh <<'SH'
#!/usr/bin/env bash
parse_config() { echo "reinvented"; }
SH
    git add scripts/cache-manager.sh

    local bare_bin="$FAKE_REPO/barebin3"
    mkdir -p "$bare_bin"
    for tool in bash git grep sed awk cut sort uniq head tail wc mktemp rm cat printf tr find comm diff; do
        if command -v "$tool" >/dev/null 2>&1; then
            ln -sf "$(command -v "$tool")" "$bare_bin/$tool" 2>/dev/null
        fi
    done

    unset AUTOSPEC_REUSE_LENS
    run env PATH="$bare_bin" bash "$LINT" --pre-commit --staged
    [[ "$output" != *"REUSE_LENS_DISABLED"* ]]
}
