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

# Resolved BEFORE the cd below, because after it a relative "$0" -- which is
# what `bash ops/ci/woodpecker-gates.sh` hands us from anywhere but the
# repository root -- no longer points at this file.
GATES_DIR="$(cd "$(dirname "$0")" && pwd)"

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
# THE RUST build-test JOB, in a sibling file.
#
# ops/ci/woodpecker-rust-gates.sh defines the eight rust-* gates that
# reproduce the GitHub Actions job `build-test`
# (.github/workflows/rust.yml) -- the check main's branch protection
# requires, and whose last GitHub run was 2026-09-17.
#
# It is a separate file because of the file-size ratchet three gates down
# from here: inline, this file would be ~875 lines against a 600-line limit,
# and the ratchet's own advice is that extracting into a sibling is the
# cheapest cut. The seam is real rather than arbitrary -- everything in that
# file is one GitHub Actions job, everything in this one is the TeamCity
# migration.
#
# Sourced, not executed, so the gates share step(), this file's
# `set -euo pipefail`, and the dispatch table at the bottom.
# shellcheck source=ops/ci/woodpecker-rust-gates.sh
. "$GATES_DIR/woodpecker-rust-gates.sh"

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
