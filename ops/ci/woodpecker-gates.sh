#!/bin/bash
# The autospec gates, as Woodpecker (ci.metabolomics.us) runs them.
#
# This file is the whole of the migration from TeamCity. Seven separate
# TeamCity build configurations --
#
#   Autospec_AccessibilityWorkstream -> accessibility
#   Autospec_ArchitectureFitness     -> architecture-fitness
#   Autospec_FileSizeRatchet         -> file-size-ratchet
#   Autospec_PythonSuites            -> python-suites
#   Autospec_SecurityWorkstream      -> security-workstream
#   Autospec_StackGuard              -> stack-guard
#   Autospec_UxUiWorkstream          -> ux-ui-workstream
#
# -- published seven independent commit statuses onto every pull request.
# Here they are steps of one pipeline, so the repository has one check.
#
# The logic lives in the repository rather than in .woodpecker.yml or on
# beegfs with the shared ci-steps, so a gate changes in the same commit as
# the code it covers, and so the whole thing is runnable by hand:
#
#   bash ops/ci/woodpecker-gates.sh              # every gate the pipeline runs
#   bash ops/ci/woodpecker-gates.sh file-size-ratchet stack-guard
#
# "every gate the pipeline runs" is six of those seven plus the eight rust-*
# gates: architecture-fitness is implemented here but is not wired in, and
# runs only when named. See the PIPELINE_GATES comment at the bottom for why.
#
# The rust-* gates are NOT from TeamCity. They reproduce the GitHub Actions
# job `build-test` (.github/workflows/rust.yml), which is the check main's
# branch protection requires and which last ran on GitHub on 2026-09-17:
#
#   bash ops/ci/woodpecker-gates.sh rust-clippy rust-workspace-test
#
# It takes no Woodpecker-specific input that it cannot default. Outside CI
# the CI_* variables are unset and every gate falls back to the same
# defaults the TeamCity steps used (base branch `main`, HEAD's own sha).
#
# ---------------------------------------------------------------------------
# WHAT THE AGENTS ACTUALLY ARE, because it decides several choices below.
#
# A Woodpecker agent here is a Slurm job on a Rocky 9 node, but the agent
# execs itself inside an Apptainer image (woodpecker/images/ci.sif) and runs
# every step as a child process, so a step sees the IMAGE, not the node:
#
#   Debian 13 (trixie), python3 = 3.13.5, git, jq, node, npm, cargo, and
#   shellcheck 0.10.0 since the image rebuild of 2026-09-22
#   NOT present: gh, python3.12
#
# No gate below runs shellcheck: none of the seven TeamCity configurations
# did, so wiring it in here would be a new check smuggled into a migration.
# `bash -n`, which two of them do run, is reproduced.
#
# There is one container per workflow, not one per step, so all the steps
# below share a workspace and a process tree. Nothing here may assume
# docker, sudo or apt.
# ---------------------------------------------------------------------------
set -euo pipefail

cd "$(git rev-parse --show-toplevel 2>/dev/null || printf '.')"

# TeamCity set these as build parameters; they are gate thresholds, not
# environment, so they are stated here where the gate that reads them is.
export MAX_LOC="${MAX_LOC:-600}"            # Autospec_FileSizeRatchet
export HARD_LOC="${HARD_LOC:-2000}"         # Autospec_FileSizeRatchet
export AUTOSPEC_PR_SIZE_STRICT="${AUTOSPEC_PR_SIZE_STRICT:-0}"   # Autospec_StackGuard

# TeamCity exposed the checked-out revision as GITHUB_SHA (env.GITHUB_SHA =
# %build.vcs.number%) because the workstream helpers stamp it into their
# ledgers. Woodpecker's equivalent is CI_COMMIT_SHA.
export GITHUB_SHA="${GITHUB_SHA:-${CI_COMMIT_SHA:-$(git rev-parse HEAD)}}"

step() { echo; echo "=== $* ==="; }

# ── The base branch, and the three shapes its name can arrive in ────────────
#
# Carried over from Autospec_FileSizeRatchet, whose comment is worth keeping:
# a base that silently collapses to `main` turns a stacked PR's one-layer
# diff into the whole accumulated stack, and the gate then measures something
# nobody asked for while still reporting green. So the collapse is deliberate
# and announced, never implicit.
#
# CI_COMMIT_TARGET_BRANCH is set by Woodpecker for pull_request events only.
# On a push it is empty, which is the "not a PR" case TeamCity spelled "N/A".
resolve_target() {
    local target="${CI_COMMIT_TARGET_BRANCH:-}"
    case "$target" in
        ""|"N/A") target="${CI_REPO_DEFAULT_BRANCH:-main}" ;;
    esac
    printf '%s' "${target#refs/heads/}"
}

# Non-fatal, as on TeamCity, and SKIPPED when the ref is already there.
#
# TeamCity fetched unconditionally because its checkout did not stage the base
# branch. Woodpecker's checkout step here does, with an explicit refspec, so
# the fetch is normally a no-op round trip -- and three gates want the same
# ref. All the steps of a workflow share ONE container, ONE workspace and ONE
# .git, and they are declared as a fan-out, so those three can be in flight at
# once: concurrent fetches writing the same refs/remotes/origin/<base> contend
# on git's ref lock and one of them fails with "cannot lock ref" for no reason
# the log explains. Checking first removes the race without removing the
# fallback: if the checkout step did not stage the ref, this still fetches it,
# and require_base below still reports a miss explicitly rather than letting
# `git merge-base` die with something unreadable.
fetch_target() {
    local target="$1"
    if git rev-parse --verify --quiet "origin/${target}" > /dev/null; then
        echo "origin/${target} already staged by the checkout step"
        return 0
    fi
    git fetch --no-tags origin "+refs/heads/${target}:refs/remotes/origin/${target}" \
        || echo "WARN: fetch of ${target} failed; relying on refs already present"
}

require_base() {
    local target="$1"
    if ! git rev-parse --verify --quiet "origin/${target}" > /dev/null; then
        echo "FATAL: origin/${target} does not exist in this checkout; the merge base cannot be computed." >&2
        echo "       The checkout step fetches it with an explicit refspec -- if it is missing, that step is wrong." >&2
        exit 1
    fi
}

# ═══════════════════════════════════════════════════════════════════════════
# THE RUST build-test JOB
#
# Everything from here to the next banner reproduces ONE GitHub Actions job,
# `build-test` in .github/workflows/rust.yml -- not a TeamCity configuration.
# It is here because `build-test` is the check `main`'s branch protection
# requires, its last GitHub run was 2026-09-17, and it is what blocks every
# pull request that TeamCity and Woodpecker both pass.
#
# Its nine GitHub steps become nine gates rather than one, for the reason the
# job's own comment on --no-fail-fast gives: when a single step covers clippy,
# the tests, catalog parity, validate and build, one long-standing failure
# hides everything behind it. Here a red `rust-workspace-test` still leaves
# `rust-catalog-parity`, `rust-validate` and `rust-build` to report for
# themselves.
#
#   GitHub step                            gate
#   Install pinned executor integration    rust-tools
#     tools + Bootstrap repository test
#     tools
#   Clippy (lint)                          rust-clippy
#   Test Linux ownership contracts         rust-ownership-contracts
#   Test workspace                         rust-workspace-test
#   Catalog count parity                   rust-catalog-parity
#   Validate repository                    rust-validate
#   Build                                  rust-build
#   Test (behaviour probes)                rust-behaviour-probes
#
# The gates share a workspace and a target directory and must therefore run in
# SEQUENCE, unlike the six workstream gates above: concurrent cargo
# invocations serialise on the target-directory lock anyway, and doing it by
# declaration makes the wait visible in the pipeline instead of hidden inside
# a step.
# ═══════════════════════════════════════════════════════════════════════════

# Versions and digests, lifted from .github/workflows/rust.yml unchanged.
# Changing one here without changing it there makes the two CIs test different
# software while both report green.
CODEX_VERSION=0.147.0
CODEX_SHA256=0246e2e773834e07f0fb5249ed6ebad12e4591e608f8c7bb97dd6a9690544c36
CODEX_BWRAP_SHA256=e73dc46e2ec7176499cb14e26c7b80b9d8e24a39cd51fe8fa0d45ddd8f6fb87c
GITLEAKS_VERSION=8.30.1
GITLEAKS_SHA256=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
TRIVY_VERSION=0.74.0
TRIVY_SHA256=2ae6fe3ee734b7fdf11335663e18c75ea12dccc76062f09f164a3b0f8be4371a

# Tools the GitHub job gets from `sudo apt-get install` or from the runner
# image, and that this image does not carry. There is no apt and no sudo
# inside ci.sif, so each one is fetched as a checksum-verified release
# artifact instead -- the same pattern the job already uses for codex,
# gitleaks and trivy, applied to the three it did not have to.
#
# gh is NOT optional: scripts/dev-bootstrap.sh's check_tools lists it and
# exits non-zero when it is absent, so the bootstrap step cannot pass without
# it. Installing it here also un-blinds stack-guard's linearity half (#4724)
# whenever a token is present; with no token `gh pr list` still fails and the
# gate stays advisory, exactly as it is today.
GH_VERSION=2.81.0
GH_SHA256=d507c82e3ae47af875b93472f39e01cbf9f85808ec0a69751e0932a43514d7ff
RIPGREP_VERSION=14.1.1
RIPGREP_SHA256=4cf9f2741e6c465ffdb7c26f38056a59e2a2544b51f7cc128ef28337eeae4d8e
# bats-core publishes no binary asset and a GitHub source-archive tarball is
# regenerated on demand, so its sha256 is not a stable pin. The tag's COMMIT
# is, and it is the stronger one: a moved tag changes it.
BATS_TAG=v1.11.1
BATS_COMMIT=b640ec3cf2c7c9cfc9e6351479261186f76eeec8
# npm's own integrity hashes cover the tarball; the exact version is the pin.
AJV_CLI_VERSION=5.0.0
LICENSE_CHECKER_VERSION=25.0.1

# ── The cargo environment, and why none of it can be left at its default ────
#
# CARGO_HOME defaults to /usr/local/cargo inside this image, which is on the
# READ-ONLY SIF. The first `cargo fetch` then dies with
#
#   error: could not create temp file ...: Read-only file system
#
# which names the filesystem and not the cause -- the same trap ci.def already
# documents for RUSTUP_HOME. Both move into the workspace, which is node-local
# ext4 on nvme and dies with the allocation. RUSTUP_HOME is deliberately left
# alone: the pinned 1.91.0 toolchain is baked into the image and re-downloading
# it per pipeline would be pure cost.
#
# CARGO_BUILD_JOBS is capped BELOW the 8 CPUs the agent asks Slurm for. An
# agent gets --mem=18G, and rustc's peak is per-codegen-unit: eight parallel
# codegen jobs over a 531k-line workspace is an OOM-kill candidate, and an
# OOM-killed run is unverified rather than red -- it reports a signal, not a
# test result. Four is the number that has held elsewhere on this cluster.
rust_env() {
    local root
    root="$(pwd)"
    CI_TOOLS="$root/.ci-tools"
    CARGO_HOME="$root/.ci-cargo"
    CARGO_TARGET_DIR="$root/target"
    export CI_TOOLS CARGO_HOME CARGO_TARGET_DIR
    export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
    export CARGO_TERM_COLOR=never
    export CARGO_NET_RETRY=3
    export PATH="$CI_TOOLS/bin:$PATH"
    mkdir -p "$CARGO_HOME" "$CI_TOOLS/bin"
}

# Fetch one archive, verify its sha256, and unpack the named members.
fetch_verified() {
    local url="$1" sha="$2" archive="$3"
    shift 3
    curl --fail --location --silent --show-error "$url" --output "$archive"
    printf '%s  %s\n' "$sha" "$archive" | sha256sum --check
    tar -xzf "$archive" -C "$CI_TOOLS/bin" "$@"
}

# ── build-test: "Install pinned executor integration tools" + "Bootstrap" ───
gate_rust_tools() {
    step "pinned executor integration tools"
    rust_env

    local tmp
    tmp="$(mktemp -d)"

    echo "-- codex ${CODEX_VERSION} (+ bwrap)"
    fetch_verified \
        "https://github.com/openai/codex/releases/download/rust-v${CODEX_VERSION}/codex-x86_64-unknown-linux-musl.tar.gz" \
        "$CODEX_SHA256" "$tmp/codex.tar.gz"
    fetch_verified \
        "https://github.com/openai/codex/releases/download/rust-v${CODEX_VERSION}/bwrap-x86_64-unknown-linux-musl.tar.gz" \
        "$CODEX_BWRAP_SHA256" "$tmp/codex-bwrap.tar.gz"
    mv -f "$CI_TOOLS/bin/codex-x86_64-unknown-linux-musl" "$CI_TOOLS/bin/codex"
    mv -f "$CI_TOOLS/bin/bwrap-x86_64-unknown-linux-musl" "$CI_TOOLS/bin/bwrap"

    echo "-- gitleaks ${GITLEAKS_VERSION}"
    fetch_verified \
        "https://github.com/gitleaks/gitleaks/releases/download/v${GITLEAKS_VERSION}/gitleaks_${GITLEAKS_VERSION}_linux_x64.tar.gz" \
        "$GITLEAKS_SHA256" "$tmp/gitleaks.tar.gz" gitleaks

    # trivy is not in Debian's repositories either, so the workflow's note --
    # that ensure-tool.sh's `apt-get install trivy` never resolves and
    # install.sh hard-verifies AUTOSPEC_EXECUTOR_SCANNERS -- holds here
    # unchanged.
    echo "-- trivy ${TRIVY_VERSION}"
    fetch_verified \
        "https://github.com/aquasecurity/trivy/releases/download/v${TRIVY_VERSION}/trivy_${TRIVY_VERSION}_Linux-64bit.tar.gz" \
        "$TRIVY_SHA256" "$tmp/trivy.tar.gz" trivy

    echo "-- gh ${GH_VERSION}"
    curl --fail --location --silent --show-error \
        "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_amd64.tar.gz" \
        --output "$tmp/gh.tar.gz"
    printf '%s  %s\n' "$GH_SHA256" "$tmp/gh.tar.gz" | sha256sum --check
    tar -xzf "$tmp/gh.tar.gz" -C "$tmp" "gh_${GH_VERSION}_linux_amd64/bin/gh"
    mv -f "$tmp/gh_${GH_VERSION}_linux_amd64/bin/gh" "$CI_TOOLS/bin/gh"

    echo "-- ripgrep ${RIPGREP_VERSION}"
    curl --fail --location --silent --show-error \
        "https://github.com/BurntSushi/ripgrep/releases/download/${RIPGREP_VERSION}/ripgrep-${RIPGREP_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
        --output "$tmp/rg.tar.gz"
    printf '%s  %s\n' "$RIPGREP_SHA256" "$tmp/rg.tar.gz" | sha256sum --check
    tar -xzf "$tmp/rg.tar.gz" -C "$tmp" "ripgrep-${RIPGREP_VERSION}-x86_64-unknown-linux-musl/rg"
    mv -f "$tmp/ripgrep-${RIPGREP_VERSION}-x86_64-unknown-linux-musl/rg" "$CI_TOOLS/bin/rg"

    # SEMGREP IS NOT INSTALLED, AND NOTHING HERE PRETENDS IT IS.
    #
    # build-test pins semgrep as a DOCKER IMAGE DIGEST
    # (semgrep/semgrep@sha256:44dd022c...) and puts a `docker run` wrapper on
    # PATH. There is no docker in this image -- ci.def refuses it deliberately
    # -- and a container runtime is the only way to run that exact artifact.
    #
    # The alternatives were considered and rejected, not overlooked:
    #   * `pip install semgrep==1.173.0` is a DIFFERENT artifact with no
    #     digest pin. Substituting it silently would mean the two CIs scan
    #     with different software while both say semgrep.
    #   * A shim on PATH that satisfies `command -v semgrep` would turn the
    #     tests that require it green by fabrication. That is the one thing a
    #     migration must never do.
    #
    # So semgrep is absent, the gates that need it are named in the PR body
    # and in the issue linked from it, and TeamCity is not involved either
    # way -- this is a GitHub Actions job, and GitHub keeps asserting it.
    echo "-- semgrep: NOT INSTALLED (pinned as a docker image; no docker here)"

    echo "-- bats ${BATS_TAG}, ajv-cli ${AJV_CLI_VERSION}, license-checker ${LICENSE_CHECKER_VERSION}"
    # The GitHub job apt-installs bats and npm-installs the other two through
    # dev-bootstrap.sh. npm's global prefix here is /usr/local, on the
    # read-only SIF, so the prefix moves into the workspace. The versions are
    # pinned; dev-bootstrap.sh installs them unpinned, and CI should not be
    # the place a tool changes without a commit.
    #
    # `npm install --prefix DIR` is a LOCAL install: it writes
    # DIR/node_modules and links the executables into
    # DIR/node_modules/.bin, not into DIR/bin the way `--global` would.
    # Symlinking them into the one directory already on PATH keeps every
    # later gate's environment a single entry, and keeps dev-bootstrap.sh's
    # `command -v ajv` answering.
    npm install --silent --no-fund --no-audit --prefix "$CI_TOOLS" \
        "ajv-cli@${AJV_CLI_VERSION}" "license-checker@${LICENSE_CHECKER_VERSION}"
    ln -sf "$CI_TOOLS/node_modules/.bin/ajv" "$CI_TOOLS/bin/ajv"
    ln -sf "$CI_TOOLS/node_modules/.bin/license-checker" "$CI_TOOLS/bin/license-checker"

    # bats from its tag, verified by COMMIT rather than by a tarball digest.
    git clone --quiet --depth 1 --branch "$BATS_TAG" \
        https://github.com/bats-core/bats-core.git "$tmp/bats-core"
    local got
    got="$(git -C "$tmp/bats-core" rev-parse HEAD)"
    if [ "$got" != "$BATS_COMMIT" ]; then
        echo "FATAL: bats-core $BATS_TAG is commit $got, expected $BATS_COMMIT" >&2
        echo "       The tag moved. Do not update the pin without reading what changed." >&2
        exit 1
    fi
    bash "$tmp/bats-core/install.sh" "$CI_TOOLS"

    rm -rf "$tmp"

    step "installed tool versions"
    # Asserted, not merely printed: a half-unpacked archive still leaves a
    # file on PATH, and the failure would otherwise surface three gates later
    # as an unexplained test failure.
    codex --version
    gitleaks version
    trivy --version | head -1
    gh --version | head -1
    rg --version | head -1
    bats --version
    # ajv and license-checker are asserted differently on purpose. Neither has
    # a usable `--version`: `ajv --version` exits 2 with a usage message, and
    # `license-checker --version` prints the version and then exits 1. Under
    # `set -e` either would fail this gate for no reason. `npm ls` is the
    # authority on what was installed anyway, and `command -v` is what
    # dev-bootstrap.sh's check_tools actually asks.
    command -v ajv
    command -v license-checker
    npm ls --prefix "$CI_TOOLS" --depth=0

    step "bootstrap repository test tools"
    # dev-bootstrap.sh is run AFTER the installs above, not instead of them.
    # It prefers apt when apt-get exists -- which it does here -- and then
    # calls `sudo apt-get`, and there is no sudo. Every install_* function in
    # it is idempotent and no-ops on a tool already on PATH, so pre-installing
    # is what makes it reach check_tools, which is the part worth having: it
    # is the repository's own statement of what a working checkout needs.
    bash scripts/dev-bootstrap.sh
}

# ── build-test: "Clippy (lint)" ─────────────────────────────────────────────
gate_rust_clippy() {
    step "clippy (workspace, all targets)"
    rust_env
    cargo clippy --workspace --all-targets
}

# ── build-test: "Test Linux ownership contracts" ────────────────────────────
gate_rust_ownership_contracts() {
    step "Linux ownership contracts"
    rust_env
    cargo test -p autospec-cli --bin autospec pidfd -- --nocapture
    cargo test -p autospec-cli --bin autospec subreaper -- --nocapture
    cargo test -p autospec-cli --bin autospec heartbeat_startup -- --nocapture
    cargo test -p autospec-cli --test claim_commands stale_startup_recovery -- --nocapture
    cargo test -p autospec-cli --bin autospec cleanup_restart -- --nocapture
}

# ── build-test: "Test workspace" ────────────────────────────────────────────
gate_rust_workspace_test() {
    step "workspace test suite"
    rust_env
    # --no-fail-fast, for the reason build-test states: without it cargo stops
    # at the first failing test binary, so one long-standing failure hides
    # every later binary. Keeping it means this gate reports the WHOLE failure
    # set in one run, which is the difference between one investigation and
    # seven.
    cargo test --workspace --no-fail-fast
}

# ── build-test: "Catalog count parity" ──────────────────────────────────────
gate_rust_catalog_parity() {
    step "catalog count parity"
    rust_env
    bash scripts/check-catalog-count-parity.sh
}

# ── build-test: "Validate repository" ───────────────────────────────────────
gate_rust_validate() {
    step "validate repository"
    rust_env
    cargo run -p autospec-cli -- validate
}

# ── build-test: "Build" ─────────────────────────────────────────────────────
gate_rust_build() {
    step "build workspace"
    rust_env
    cargo build --workspace
}

# ── build-test: "Test" (the exact behaviour probes) ─────────────────────────
#
# Carried over whole, including the counted assertion. `cargo test <name>` is
# a SUBSTRING filter: a probe whose test was renamed matches nothing, and a
# run of zero tests exits 0 and reads as a pass. Counting one "... ok" line
# is what makes a vanished test a failure instead of a silent hole -- the same
# class of mistake as a mutant run that executed no test.
run_exact() {
    local behavior_test="$1" proof_marker="${2:-}" behavior_output behavior_status
    set +e
    behavior_output="$(cargo test -p autospec-cli --bin autospec "$behavior_test" -- --exact --nocapture 2>&1)"
    behavior_status=$?
    set -e
    printf '%s\n' "$behavior_output"
    if [ "$behavior_status" -ne 0 ]; then
        return "$behavior_status"
    fi
    test "$(printf '%s\n' "$behavior_output" | grep -F -c "test $behavior_test ... ok")" -eq 1
    if [ -n "$proof_marker" ]; then
        test "$(printf '%s\n' "$behavior_output" | grep -F -c "$proof_marker")" -eq 1
    fi
}

gate_rust_behaviour_probes() {
    step "autospec-core lib tests and portable ownership probes"
    rust_env
    cargo test -p autospec-core --lib
    run_exact 'commands::claim::heartbeat_portable::tests::publication_is_idempotent_but_rejects_another_generation'
    run_exact 'commands::autonomous::executor_bridge::tests::adoption_cleanup::autonomous_executor_bridge_pidfd_adoption_requires_full_exec_identity'
    run_exact 'commands::autonomous::executor_bridge::portability::supported_host_tests::supported_host_retires_predecessor_runs_noop_and_publishes_terminal_receipt'
}

# ── Autospec_AccessibilityWorkstream ────────────────────────────────────────
gate_accessibility() {
    step "accessibility workstream contract"
    bash -n scripts/accessibility-workstream.sh
    bash scripts/accessibility-workstream.sh validate-design-doc \
        --doc docs/runbooks/accessibility-workstream.md

    # A synthetic two-theme ledger, then the gate over it. This asserts the
    # helper's record/gate contract end to end without needing a browser,
    # axe, pa11y or Lighthouse -- none of which exist on these agents.
    local work ledger
    work="$(mktemp -d)"
    ledger="$work/scans.jsonl"
    bash scripts/accessibility-workstream.sh record-scan \
        --ledger "$ledger" --commit "$GITHUB_SHA" --theme light \
        --axe-violations 0 --pa11y-errors 0 --lighthouse-a11y 100 \
        --ibm-violations 0 --auto-fixable 0 --judgment-findings 0
    bash scripts/accessibility-workstream.sh record-scan \
        --ledger "$ledger" --commit "$GITHUB_SHA" --theme dark \
        --axe-violations 0 --pa11y-errors 0 --lighthouse-a11y 100 \
        --ibm-violations 0 --auto-fixable 0 --judgment-findings 0
    bash scripts/accessibility-workstream.sh gate --ledger "$ledger" --commit "$GITHUB_SHA"
    rm -rf "$work"

    step "accessibility deterministic gate (real scans, when present)"
    if [ -f .autospec/accessibility/scans.jsonl ]; then
        bash scripts/accessibility-workstream.sh gate \
            --ledger .autospec/accessibility/scans.jsonl \
            --commit "$GITHUB_SHA" \
            --findings-out .autospec/accessibility/findings.jsonl
    else
        echo "No accessibility scans present; contract validation covered helper/runbook wiring."
    fi
}

# ── Autospec_UxUiWorkstream ─────────────────────────────────────────────────
gate_ux_ui() {
    step "UX/UI workstream contract"
    bash -n scripts/ux-ui-workstream.sh
    bash scripts/ux-ui-workstream.sh validate-design-doc \
        --doc docs/runbooks/ux-ui-workstream.md

    local work ledger
    work="$(mktemp -d)"
    ledger="$work/snapshots.jsonl"
    bash scripts/ux-ui-workstream.sh record-snapshot \
        --ledger "$ledger" --commit "$GITHUB_SHA" --theme light \
        --lcp-ms 2200 --inp-ms 150 --cls 0.05 --lighthouse-performance 94 \
        --token-violations 0 --visual-diff-pct 0.04 --console-errors 0 \
        --failed-requests 0 --tap-target-violations 0 --horizontal-overflow 0
    bash scripts/ux-ui-workstream.sh record-snapshot \
        --ledger "$ledger" --commit "$GITHUB_SHA" --theme dark \
        --lcp-ms 2300 --inp-ms 170 --cls 0.07 --lighthouse-performance 93 \
        --token-violations 0 --visual-diff-pct 0.05 --console-errors 0 \
        --failed-requests 0 --tap-target-violations 0 --horizontal-overflow 0
    bash scripts/ux-ui-workstream.sh gate --ledger "$ledger" --commit "$GITHUB_SHA"
    rm -rf "$work"

    step "UX/UI deterministic gate (real snapshots, when present)"
    if [ -f .autospec/ux-ui/snapshots.jsonl ]; then
        bash scripts/ux-ui-workstream.sh gate \
            --ledger .autospec/ux-ui/snapshots.jsonl \
            --commit "$GITHUB_SHA" \
            --regressions-out .autospec/ux-ui/regressions.jsonl
    else
        echo "No UX/UI snapshots present; contract validation covered helper/runbook wiring."
    fi
}

# ── Autospec_ArchitectureFitness ────────────────────────────────────────────
gate_architecture_fitness() {
    step "architecture fitness gates"

    # Drop latency_budget_validate_fast on a shared cluster node.
    #
    # Carried over from Autospec_ArchitectureFitness verbatim, and for the
    # same reason. That gate is `command_max_ms` on /usr/bin/true with a 50ms
    # budget -- it measures process-spawn overhead, not anything about
    # autospec's code. On a GitHub runner it lands at ~2ms; on a shared Slurm
    # node with quobyte/NFS scratch it measured 56ms (TeamCity build 30226)
    # and failed. That is environment noise, and a red non-advisory check
    # here stalls autospec's auto-merger.
    #
    # Measured again on this backend 2026-09-22 (inside ci.sif on
    # kvm-node-5) it happened to land at 7ms -- which is exactly the problem:
    # the number is a property of whichever node the Slurm scheduler picked
    # and how loaded it was, not of the commit under test. A gate that
    # depends on that is a coin flip, and one whose outcome cannot be
    # reproduced from the diff. It stays out.
    #
    # The budget is calibrated for a GitHub runner and is asserted nowhere
    # now that .github/workflows/architecture-fitness.yml is retired
    # (1efca71f). See the issue linked from the PR that added this file.
    awk '
      /^  - id: latency_budget_validate_fast$/ { skip=1; next }
      /^  - id: / { skip=0 }
      !skip { print }
    ' .autospec/architecture-fitness.yml > .af-registry-woodpecker.yml

    # The count is the assertion that the filter removed exactly one gate
    # rather than silently eating the rest of the registry -- an awk skip
    # that matched too much would otherwise produce a small, green,
    # meaningless run. TeamCity printed this number; here it is checked.
    local before after
    before="$(grep -c '^  - id: ' .autospec/architecture-fitness.yml)"
    after="$(grep -c '^  - id: ' .af-registry-woodpecker.yml)"
    echo "registry gates: ${before} -> ${after} (latency_budget_validate_fast filtered out)"
    if [ "$after" -ne "$((before - 1))" ]; then
        echo "FATAL: the registry filter removed $((before - after)) gates, expected exactly 1." >&2
        echo "       Either latency_budget_validate_fast was renamed, or the awk skip is wrong." >&2
        exit 1
    fi

    bash scripts/architecture-fitness.sh run --registry .af-registry-woodpecker.yml
}

# ── Autospec_PythonSuites ───────────────────────────────────────────────────
gate_python_suites() {
    step "python suites (context-monitor)"

    # THE INTERPRETER IS 3.13 HERE, NOT THE 3.12 TEAMCITY ASSERTED.
    #
    # TeamCity asserted `sys.version_info[:2] == (3, 12)` rather than
    # installing a version, because its Ubuntu 24.04 agent image shipped
    # 3.12.3 as the system python and a silent drift would change what the
    # suites exercise. The assertion, not the number, is the point.
    #
    # This backend's image is Debian 13 (trixie), whose system python is
    # 3.13.5. There is no 3.12 in the image and no way to add one from a
    # step (no apt, no sudo, no docker). So the number moves and the
    # assertion stays: the suites are 198 tests and all 198 pass on 3.13.5
    # (measured 2026-09-22 inside ci.sif), and if the image is rebuilt onto
    # a different python this gate says so instead of quietly testing it.
    #
    # The 3.12 pin now exists nowhere -- .github/workflows/python.yml was
    # retired in 1efca71f. See the issue linked from the PR that added this
    # file for whether 3.12 should go back into the image.
    python3 -c 'import sys; assert sys.version_info[:2] == (3, 13), sys.version'
    python3 --version

    # A venv with pinned-by-name test deps, as TeamCity did, rather than the
    # image's own pytest. The image happens to carry pytest 9.1.1 and pyyaml
    # 6.0.3, but relying on that makes an image rebuild able to change the
    # test runner under a green check without a commit.
    rm -rf .ci-venv
    python3 -m venv .ci-venv
    .ci-venv/bin/pip install --quiet --upgrade pip
    .ci-venv/bin/pip install --quiet pytest pyyaml

    export PYTHONPATH="packages/autospec_context_monitor${PYTHONPATH:+:$PYTHONPATH}"
    .ci-venv/bin/python -m pytest packages tests -q -p no:cacheprovider
}

# Extensions the file size rules do not apply to.
skip_ext() {
    case "$1" in
        *.md|*.txt|*.json|*.yaml|*.yml|*.diff|*.lock|*.snap) return 0 ;;
        *) return 1 ;;
    esac
}

# ── Autospec_FileSizeRatchet ────────────────────────────────────────────────
#
# A ratchet, not an absolute limit: a file already over MAX_LOC may be edited
# and shrunk, but not grown. Relocation -- a change that removes at least as
# many lines as it adds -- is the one case where a new oversized file is
# tolerated, because rejecting it would preserve the larger monolith.
gate_file_size_ratchet() {
    step "file size ratchet"
    local target base
    target="$(resolve_target)"
    echo "merge-base target: $target"
    fetch_target "$target"
    require_base "$target"

    base="$(git merge-base "origin/${target}" HEAD)"
    echo "merge base: $base"

    # Net line delta across the whole change. Taken from --numstat rather than
    # by differencing file lengths, because git reports a move as a rename: the
    # source path never appears on its own, so length differencing counts the
    # destination as wholly new and a relocation reads as pure addition.
    local net
    net="$(git diff --numstat "$base"...HEAD | awk '
      {
        path = $3
        # Renames arrive as "old => new" or "dir/{old => new}"; judge the target.
        if (index(path, "=>")) { sub(/^.*=> ?/, "", path); gsub(/[}]/, "", path) }
        if (path ~ /\.(md|txt|json|ya?ml|diff|lock|snap)$/) next
        if ($1 == "-" || $2 == "-") next   # binary
        net += $1 - $2
      }
      END { print net + 0 }
    ')"
    echo "net line delta: $net"

    # A change that removes at least as many lines as it adds is a relocation,
    # not new material: a feature cannot hide in it, because features add lines.
    local is_relocation=0
    [ "$net" -le 0 ] && is_relocation=1

    local fail=0 f after before
    # ACMR: added, copied, modified, renamed. Deletions cannot grow a file.
    for f in $(git diff --name-only --diff-filter=ACMR "$base"...HEAD); do
        [ -f "$f" ] || continue
        skip_ext "$f" && continue

        after="$(wc -l < "$f" | tr -d ' ')"
        if git cat-file -e "$base:$f" 2>/dev/null; then
            before="$(git show "$base:$f" | wc -l | tr -d ' ')"
        else
            before=""
        fi

        # New file: judged against the threshold directly, unless it holds
        # relocated code -- see the relocation note above.
        if [ -z "$before" ]; then
            if [ "$after" -gt "$MAX_LOC" ]; then
                if [ "$is_relocation" -eq 1 ]; then
                    echo "WARNING: $f is a $after line extraction, over the $MAX_LOC limit; allowed because the change is a net removal, but split it further"
                else
                    echo "FILE_SIZE:$f: new file is $after lines (limit $MAX_LOC); split it before landing"
                    fail=1
                fi
            fi
            continue
        fi

        # Existing file at or under the threshold: unconstrained.
        [ "$after" -le "$MAX_LOC" ] && continue

        # Existing oversized file: may shrink or hold, may not grow.
        if [ "$after" -gt "$before" ]; then
            echo "FILE_SIZE:$f: grew from $before to $after lines and is over the $MAX_LOC limit"
            echo "  an oversized file may be edited and shrunk, but not made longer"
            fail=1
        else
            echo "ok (not grown): $f  $before -> $after"
        fi

        # Past the hard ceiling, say so even when the change shrinks the file,
        # so the worst offenders stay visible in every PR that touches them.
        if [ "$after" -gt "$HARD_LOC" ]; then
            echo "WARNING: $f is $after lines, past the $HARD_LOC hard ceiling; it needs splitting"
        fi
    done

    if [ "$fail" -ne 0 ]; then
        echo ""
        echo "A file over $MAX_LOC lines got longer. Move code out rather than adding to it;"
        echo "extracting an inline #[cfg(test)] mod tests into a sibling file is usually the"
        echo "cheapest first cut and carries no production risk."
        exit 1
    fi
    echo "file size ratchet: OK"
}

# ── Autospec_StackGuard ─────────────────────────────────────────────────────
gate_stack_guard() {
    step "stack guard (single layer)"
    local target
    target="$(resolve_target)"
    echo "stack base target: $target"
    fetch_target "$target"
    require_base "$target"

    # --default-branch is passed explicitly because the script otherwise
    # resolves it with `gh repo view`, and gh is not in this image. Without
    # the flag the resolution falls through to the literal "main", which is
    # right today and silently wrong the day the default branch is renamed.
    #
    # PARTIAL REPRODUCTION, stated here and in the PR body: the linearity
    # half of this gate calls `gh pr list` to check that a PR's base is the
    # head of another open PR. With no gh, that call fails, the open-head
    # list is empty, and any PR whose base is NOT the default branch is
    # reported non-linear -- advisory at AUTOSPEC_PR_SIZE_STRICT=0, which is
    # the setting TeamCity used, so it does not block. The per-layer PR_SIZE
    # half is unaffected and fully reproduced.
    bash scripts/stack-guard.sh \
        --base "origin/${target}" \
        --head HEAD \
        --default-branch "${CI_REPO_DEFAULT_BRANCH:-main}"
}

# ── Autospec_SecurityWorkstream ─────────────────────────────────────────────
gate_security_workstream() {
    step "security workstream"

    # TeamCity discriminated on teamcity.build.branch.is_default: the default
    # branch scans the whole tree, anything else scans the PR diff. That is
    # the same split the retired GitHub workflow made on github.event_name
    # (cron -> tree, pull_request -> diff). Reproduced on Woodpecker's two
    # variables, and deliberately NOT on CI_PIPELINE_EVENT alone: a push to a
    # feature branch is not the default branch, and treating it as one would
    # turn a cheap diff scan into a full tree scan on every push.
    local mode target
    if [ "${CI_PIPELINE_EVENT:-}" != "pull_request" ] \
       && [ "${CI_COMMIT_BRANCH:-}" = "${CI_REPO_DEFAULT_BRANCH:-main}" ]; then
        mode="tree"
    else
        mode="pr"
    fi
    echo "scan mode: $mode"

    mkdir -p .autospec/security
    if [ "$mode" = "pr" ]; then
        target="$(resolve_target)"
        echo "diff base: origin/${target}"
        fetch_target "$target"
        require_base "$target"
        bash scripts/security-workstream.sh scan --mode pr --base "origin/${target}" \
            --out .autospec/security/security-ranked.jsonl
    else
        bash scripts/security-workstream.sh scan --mode tree \
            --out .autospec/security/security-ranked.jsonl
    fi

    step "dashboard security headers"
    bash scripts/security-workstream.sh check-headers --headers-file docs/site/_headers

    step "block P0/P1 security findings"
    if grep -Eq '"priority":"P[01]"' .autospec/security/security-ranked.jsonl; then
        cat .autospec/security/security-ranked.jsonl
        exit 1
    fi
    echo "no P0/P1 findings"
}

# ── Dispatch ────────────────────────────────────────────────────────────────
# The gates the pipeline runs, and therefore what a bare invocation runs.
#
# architecture-fitness is DELIBERATELY NOT IN THIS LIST, though it is fully
# implemented above and runs when named:
#
#   bash ops/ci/woodpecker-gates.sh architecture-fitness
#
# Its rust_core_cli_direction gate has been failing on main continuously --
# 73 occurrences against a threshold of 0 -- and TeamCity has published it red
# on every recent pull request (#4720, #4721, #4722 all merged red). As one
# status of seven that was survivable; as a step of the one check that now
# covers the repo it would be a permanent red that buries the six working
# gates. TeamCity keeps asserting it until the debt is cleared. See
# docs/runbooks/woodpecker-ci.md.
#
# The rust-* gates reproduce the GitHub Actions `build-test` job and are in
# the list: they are the coverage main's branch protection asks for. They run
# in sequence, in the order below, because they share one target directory.
PIPELINE_GATES="accessibility file-size-ratchet python-suites security-workstream stack-guard ux-ui-workstream
                rust-tools rust-clippy rust-ownership-contracts rust-workspace-test
                rust-catalog-parity rust-validate rust-build rust-behaviour-probes"
KNOWN_GATES="$PIPELINE_GATES architecture-fitness"

run_gate() {
    case "$1" in
        accessibility)        gate_accessibility ;;
        architecture-fitness) gate_architecture_fitness ;;
        file-size-ratchet)    gate_file_size_ratchet ;;
        python-suites)        gate_python_suites ;;
        security-workstream)  gate_security_workstream ;;
        stack-guard)          gate_stack_guard ;;
        ux-ui-workstream)     gate_ux_ui ;;
        rust-tools)               gate_rust_tools ;;
        rust-clippy)              gate_rust_clippy ;;
        rust-ownership-contracts) gate_rust_ownership_contracts ;;
        rust-workspace-test)      gate_rust_workspace_test ;;
        rust-catalog-parity)      gate_rust_catalog_parity ;;
        rust-validate)            gate_rust_validate ;;
        rust-build)               gate_rust_build ;;
        rust-behaviour-probes)    gate_rust_behaviour_probes ;;
        *)
            echo "unknown gate: $1" >&2
            echo "known gates: $KNOWN_GATES" >&2
            exit 2
            ;;
    esac
}

main() {
    echo "commit:  ${CI_COMMIT_SHA:-$(git rev-parse HEAD)}"
    echo "event:   ${CI_PIPELINE_EVENT:-<local>}"
    echo "gates:   $*"
    local g t0 t1
    for g in "$@"; do
        t0=$(date +%s)
        run_gate "$g"
        t1=$(date +%s)
        # Printed per gate because the rust gates are minutes rather than
        # seconds and the pipeline's cost is now a thing worth watching: a
        # gate that doubles should be visible in its own log, not only in a
        # wall-clock number on the server.
        echo "ELAPSED ${g}: $((t1 - t0))s"
    done
    echo
    echo "GATES PASSED: $*"
}

# shellcheck disable=SC2086  # deliberate word splitting: a list of gate names
[ "$#" -gt 0 ] || set -- $PIPELINE_GATES

# The run goes through a PIPELINE, not `exec > >(tee ...)`.
#
# Process substitution does not make the shell wait for the reader: a script
# that fails in seconds exits before tee drains its pipe, and the agent
# records nothing at all. That is not hypothetical -- it is why the first two
# builds of the inferweave-slurm gate returned a null log, twice, while its
# minutes-long run flushed fine. A dropped log on a fast failure is the worst
# case: it loses exactly the runs that need explaining. The journal path is
# printed FIRST, before anything can fail, so a lost log is still readable
# from the node.
#
# A pipeline is waited on, so the output survives. PIPESTATUS carries the
# body's status past tee, which would otherwise mask it with its own.
journal=""
for candidate in "${WOODPECKER_JOURNAL_DIR:-}" /home/wohlgemuth/woodpecker/logs; do
    [ -n "$candidate" ] || continue
    if mkdir -p "$candidate" 2>/dev/null && [ -w "$candidate" ]; then
        journal="$candidate/autospec-gates-${CI_COMMIT_SHA:-local}-$(date +%s)-$1.log"
        break
    fi
done

if [ -n "$journal" ]; then
    echo "journal: $journal"
    main "$@" 2>&1 | tee -a "$journal"
    exit "${PIPESTATUS[0]}"
fi
main "$@"
