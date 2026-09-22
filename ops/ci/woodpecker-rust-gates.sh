#!/bin/bash
# The GitHub Actions `build-test` job, as Woodpecker runs it.
#
# SOURCED BY ops/ci/woodpecker-gates.sh, never run on its own -- it uses that
# file's step() helper and its `set -euo pipefail`, and it is that file which
# dispatches gate names. Run a gate the documented way:
#
#   bash ops/ci/woodpecker-gates.sh rust-clippy rust-workspace-test
#
# It is a SEPARATE FILE only because of the file-size ratchet this repository
# gates on: woodpecker-gates.sh is under the 600-line limit and adding these
# 300-odd lines to it would push it over, which the ratchet refuses -- the
# gate's own advice being that extracting into a sibling file is the cheapest
# cut. The split follows a real seam: everything here is one GitHub Actions
# job, and everything there is the TeamCity migration.

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
#   Catalog count parity                   rust-catalog-parity
#   Build                                  rust-build
#   Test (behaviour probes)                rust-behaviour-probes
#   Validate repository                    rust-validate       \ parallel,
#   Test workspace                         rust-workspace-test /  and last
#
# THE ORDER IS NOT build-test's, and that is the point. On GitHub the
# workspace suite is fourth and `Validate repository` seventh, so a red suite
# means validate and build never run at all -- on main they had not executed
# in CI for as long as one test had been failing, which is the job's own
# --no-fail-fast complaint one level up. Here the six cheap gates run first
# and the two long ones run in parallel at the end, so a red one never costs
# the report of the other. NOTHING IS REORDERED WITHIN A GATE.
#
# The cheap gates are a chain rather than a fan-out because they share one
# cargo target directory: run concurrently they would only queue on cargo's
# build lock, with the wait hidden inside a step instead of shown in the
# pipeline. The two leaves are safe in parallel because everything they need
# is already compiled by the gates before them.
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
# yq is preinstalled on a GitHub runner and is in install.sh's
# AUTOSPEC_SYSTEM_TOOLS. Without it `autospec validate` fails
# check_autospec_fleet_enabled_false and check_autospec_sweep_enabled_false:
# both bats suites drive scripts/fleet-run.sh, which reads autospec-fleet.yml
# with yq, and the suite reports only `[ "$status" -eq 0 ]' failed.
YQ_VERSION=4.47.1
YQ_SHA256=7583d471d9bfe88e32005e9d287952382df0469135f691e044443f610d707f4d
# bats-core publishes no binary asset and a GitHub source-archive tarball is
# regenerated on demand, so its sha256 is not a stable pin. The tag's COMMIT
# is, and it is the stronger one: a moved tag changes it.
BATS_TAG=v1.11.1
BATS_COMMIT=b640ec3cf2c7c9cfc9e6351479261186f76eeec8
# npm's own integrity hashes cover the tarball; the exact version is the pin.
AJV_CLI_VERSION=5.0.0
LICENSE_CHECKER_VERSION=25.0.1

# ── The environment, and why none of it can be left at its default ──────────
#
# THREE THINGS THIS BACKEND GETS WRONG FOR THIS SUITE, each of which was
# measured as a specific set of failing tests on pipeline 7 rather than
# guessed at.
#
# 1. CARGO_HOME defaults to /usr/local/cargo inside this image, which is on
#    the READ-ONLY SIF. The first `cargo fetch` dies with
#
#      error: could not create temp file ...: Read-only file system
#
#    which names the filesystem and not the cause -- the same trap ci.def
#    already documents for RUSTUP_HOME. It moves to node-local scratch.
#
#    NOT into the workspace, which is where it was first put and where it
#    broke three repository-scanning gates: `.ci-cargo/registry/src/...`
#    holds the unpacked sources of 189 crates, and the shell ratchet, the
#    deadline ratchet and the block-expansion check all walk the repository
#    tree. They then counted crc-catalog's generate_tests.sh and flume's
#    tests as autospec's own -- "shell ratchet DRIFTED: 228530 counted lines
#    against an allowlist of 227120". Anything a gate writes into the
#    checkout is repository content as far as this repository's own gates are
#    concerned. RUSTUP_HOME is deliberately left alone: the pinned 1.91.0
#    toolchain is baked into the image and re-downloading it per pipeline
#    would be pure cost.
#
# 2. HOME IS UNDER /tmp, AND AUTOSPEC REFUSES TO RUN FROM /tmp.
#
#    The agent puts a whole workflow -- workspace and home -- under
#    /tmp/woodpecker-local-<n>/. harness.rs::temporary_path() treats /tmp,
#    /var/tmp, /private/tmp, /private/var/tmp, /var/folders and $TMPDIR as
#    temporary storage and safe_executable() refuses an executor harness
#    found there:
#
#      executor harness is configured through temporary storage:
#      /tmp/woodpecker-local-3053957323/home/.local/share/autospec-launch-tests/...
#
#    That is a real security property of the product -- a harness that can be
#    swapped between validation and exec is the whole point of the trusted-
#    executable check -- and eleven tests assert it. It is asserted about
#    $HOME, so a harness installed anywhere cannot rescue it: the tests build
#    their fixtures under HOME. GitHub Actions has HOME=/home/runner and
#    never meets the rule.
#
#    So CI gets a home that is not temporary. The RULE IS NOT RELAXED and no
#    test is skipped for it; the environment is made to match what the
#    product requires of a real one.
#
# 3. CARGO_BUILD_JOBS is capped BELOW the 8 CPUs the agent asks Slurm for. An
#    agent gets --mem=18G, and rustc's peak is per-codegen-unit: eight
#    parallel codegen jobs over a 531k-line workspace is an OOM-kill
#    candidate, and an OOM-killed run is unverified rather than red -- it
#    reports a signal, not a test result. Four is the number that has held
#    elsewhere on this cluster.
#
# CARGO_TARGET_DIR stays `target/` inside the workspace, as it is on GitHub:
# it is gitignored and the repository scans already skip it.

# The root under which each pipeline gets its non-temporary HOME. On the
# operator path the agent binds, beside the gate journals, and overridable so
# this is not a hard-coded cluster path for anyone else.
AUTOSPEC_CI_HOME_ROOT="${AUTOSPEC_CI_HOME_ROOT:-/home/wohlgemuth/woodpecker/ci-home}"

# Move HOME off temporary storage, and ONLY when it is on it. A developer
# running `bash ops/ci/woodpecker-gates.sh rust-clippy` already has a real
# home and must keep it -- silently relocating someone's HOME would be a
# far worse surprise than a slow gate.
rust_home() {
    case "${HOME:-/tmp}" in
        /tmp|/tmp/*|/var/tmp|/var/tmp/*|/private/tmp/*|/private/var/tmp/*|/var/folders/*) ;;
        *) return 0 ;;
    esac
    if ! mkdir -p "$AUTOSPEC_CI_HOME_ROOT" 2> /dev/null || [ ! -w "$AUTOSPEC_CI_HOME_ROOT" ]; then
        echo "WARN: $AUTOSPEC_CI_HOME_ROOT is not writable; HOME stays at ${HOME:-<unset>}," >&2
        echo "      and the eleven trusted-harness tests will fail on temporary storage." >&2
        return 0
    fi
    # Pipelines are numbered monotonically, so this directory is never shared
    # with a concurrent run. Pruning old ones is a separate, announced step in
    # gate_rust_tools -- not here, which every gate calls.
    HOME="$AUTOSPEC_CI_HOME_ROOT/autospec-${CI_PIPELINE_NUMBER:-$$}"
    mkdir -p "$HOME"
    export HOME
    echo "HOME relocated off temporary storage: $HOME"
}

# Old per-pipeline homes, pruned ONCE per pipeline and never silently.
#
# This deletes under a path the operator owns, so it says what it is about to
# remove first, matches only the directories rust_home creates, and only ones
# older than two days -- an agent's walltime is four hours, so nothing live
# can match. Set AUTOSPEC_CI_HOME_PRUNE=0 to turn it off and prune by hand.
prune_ci_homes() {
    [ "${AUTOSPEC_CI_HOME_PRUNE:-1}" = "1" ] || { echo "ci-home prune: disabled"; return 0; }
    [ -d "$AUTOSPEC_CI_HOME_ROOT" ] || return 0
    local stale
    stale="$(find "$AUTOSPEC_CI_HOME_ROOT" -mindepth 1 -maxdepth 1 -type d \
        -name 'autospec-*' -mtime +2 2> /dev/null || true)"
    if [ -z "$stale" ]; then
        echo "ci-home prune: nothing older than 2 days under $AUTOSPEC_CI_HOME_ROOT"
        return 0
    fi
    echo "ci-home prune: removing"
    printf '%s\n' "$stale" | sed 's/^/  /'
    printf '%s\n' "$stale" | while IFS= read -r dir; do
        [ -n "$dir" ] && rm -rf "$dir"
    done
}

rust_env() {
    rust_home
    # $HOME/.local/bin is exactly where build-test puts its pinned tools, and
    # it is outside the checkout, which is what keeps the repository scans
    # measuring this repository.
    CI_TOOLS="$HOME/.local"
    # Node-local scratch, outside the checkout: cargo has no opinion about
    # temporary storage, and the registry is read hard during a build.
    CARGO_HOME="${TMPDIR:-/tmp}/autospec-ci-cargo-${CI_PIPELINE_NUMBER:-$$}"
    CARGO_TARGET_DIR="$(pwd)/target"
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
    prune_ci_homes

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
    echo "-- yq ${YQ_VERSION}"
    curl --fail --location --silent --show-error \
        "https://github.com/mikefarah/yq/releases/download/v${YQ_VERSION}/yq_linux_amd64.tar.gz" \
        --output "$tmp/yq.tar.gz"
    printf '%s  %s\n' "$YQ_SHA256" "$tmp/yq.tar.gz" | sha256sum --check
    tar -xzf "$tmp/yq.tar.gz" -C "$tmp" ./yq_linux_amd64
    mv -f "$tmp/yq_linux_amd64" "$CI_TOOLS/bin/yq"

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
    yq --version
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

