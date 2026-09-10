#!/usr/bin/env bash
# autospec-guarded-merge.sh — fused blast-radius domain fence + admin merge.
#
# Runs the blast-radius classifier on a PR's ACTUAL changed files immediately
# before merging. If the diff touches a configured fenced surface and no
# override label is present, it quarantines the PR (applies the human-review
# label, comments the fenced surfaces, and refuses to merge). Otherwise it
# performs the admin squash-merge.
#
# This is the merge-time, per-diff enforcement point that the predictive
# selection-time fence (autonomous-prioritize.sh) and the tier-level premerge
# gate do not cover. Callers in /autospec-run invoke this INSTEAD of a bare
# `gh pr merge --admin`, so "merge without the fence check" requires
# deliberately bypassing the wrapper rather than merely omitting a prose step.
#
# NOTE: this is implementer-honored (soft) — the genuinely unbypassable fence
# is branch protection + a required status check (CI). This wrapper is the
# strongest per-diff fence available without CI.
#
# Usage:
#   autospec-guarded-merge.sh --pr N --repo OWNER/REPO
#       [--fenced-surfaces FILE]     # default: guardrails resolves .autospec/*
#       [--override-label LABEL]     # default: autospec:fenced-approved
#       [--human-label LABEL]        # default: autospec:needs-human
#       [--merge-args "ARGS"]        # default: --admin --squash --delete-branch
#       [--no-require-checks]        # skip the CI-conclusion gate (default: on)
#       [--checks-timeout SECS]      # default: 1800
#       [--checks-poll SECS]         # default: 30
#       [--verify-evidence FILE]     # full-suite evidence gate (issue #3523)
#       [--build-dir DIR]            # checkout the local-build gate builds (default: cwd)
#       [--no-require-local-build]   # skip the local buildability gate (default: on)
#
# CI-conclusion gate (issue #3220). `main` carries no branch protection, so no
# check is "required" and a PR whose checks are pending or failing reports
# mergeStateStatus UNSTABLE. The Phase 4 loop treats UNSTABLE as ready and
# breaks straight to merge, so its `wait_for_ci_green` never runs on the normal
# path — #3148 and #3216 both merged before their run reported, and #3148's
# `build-test` was already failing. Enforcing it here, at the chokepoint every
# auto-implement merge routes through, makes "merge before CI reports" require
# deliberately passing --no-require-checks rather than merely reaching the merge
# by a path whose prose forgot to wait.
#
# Local buildability gate (issue #4108). The CI-conclusion gate above reads
# what CI reports; this gate reads what the compiler says, locally, at merge
# time: `cargo build --workspace --all-targets` against the PR's EXACT head
# commit, with the exit status as the verdict — never a scan of output text
# for "FAILED" (the #4105 failure mode, where a broken build was "counted" as
# green because no line matched). Default-on; warn-skip (the CI gate remains
# the authority) when no local build of the head is possible — no Cargo.toml
# at the build dir, not a git worktree, no cargo on PATH, or the head commit
# not present locally (no network fetch is attempted: in production the
# monitor merges from the worktree the implementer pushed, so the head is
# already there). An actual failed build of the head commit fails closed.
# --build-dir DIR points the gate at a known checkout and makes those
# prerequisites mandatory (an operator error then fails closed);
# --no-require-local-build is the explicit opt-out. The compensating
# controls that make "no branch protection on main" survivable are
# documented in docs/runbooks/compensating-controls-main-protection.md.
#
# Full-suite evidence gate (issue #3523). "The full suite passed, on the
# commit being merged" was enforced only by prose; nothing recorded that the
# suite ran, and nothing at merge time read which commit it ran against. This
# gate closes that: with --verify-evidence FILE, a file whose head_sha no
# longer names the PR head OID, or whose status is non-zero, always refuses
# the merge. An ABSENT file warns on stderr (verify-evidence-absent) and
# merges, so the gate ships inert until Phase 4 records evidence — silence and
# staleness must not read alike. An unparseable file fails closed (exit 2).

# Exit codes:
#   0  merged (allowed, or fenced-but-overridden)
#   1  refused — NOT merged (fenced surface without override, non-advisory
#      checks not green, stale / failing full-suite evidence, or a local
#      build failure of the PR head commit)
#   2  invocation / classifier / evidence-parse error — fail-closed, NOT merged
#
# Engineering rules (AGENTS.md): set -euo pipefail; if/then/fi (no one-sided
# && short-circuits); no RETURN traps (inline cleanup).

set -euo pipefail

PR=""
REPO=""
FENCED_SURFACES=""
OVERRIDE_LABEL="autospec:fenced-approved"
HUMAN_LABEL="autospec:needs-human"
MERGE_ARGS="--admin --squash --delete-branch"
REQUIRE_CHECKS=1
CHECKS_TIMEOUT=1800
CHECKS_POLL=30
VERIFY_EVIDENCE=""
BUILD_DIR="."
_BUILD_DIR_EXPLICIT=0
REQUIRE_LOCAL_BUILD=1

_die() {
    printf 'autospec-guarded-merge: %s\n' "$1" >&2
    exit 2
}

_warn() {
    printf 'autospec-guarded-merge: WARNING: %s\n' "$1" >&2
}

# ensure_label — same idempotent convention as
# autonomous-promote-open-issues.sh's ensure_label(): create the label if
# missing (never force a recolor of a pre-existing repo label), never fail
# the caller.
ensure_label() {
    # $1 = label name, $2 = repo (OWNER/REPO)
    gh label create "$1" --repo "$2" >/dev/null 2>&1 || true
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr) PR="${2:-}"; shift 2 ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        --fenced-surfaces) FENCED_SURFACES="${2:-}"; shift 2 ;;
        --override-label) OVERRIDE_LABEL="${2:-}"; shift 2 ;;
        --human-label) HUMAN_LABEL="${2:-}"; shift 2 ;;
        --merge-args) MERGE_ARGS="${2:-}"; shift 2 ;;
        --no-require-checks) REQUIRE_CHECKS=0; shift 1 ;;
        --checks-timeout) CHECKS_TIMEOUT="${2:-}"; shift 2 ;;
        --checks-poll) CHECKS_POLL="${2:-}"; shift 2 ;;
        --verify-evidence) VERIFY_EVIDENCE="${2:-}"; shift 2 ;;
        --build-dir) BUILD_DIR="${2:-}"; _BUILD_DIR_EXPLICIT=1; shift 2 ;;
        --no-require-local-build) REQUIRE_LOCAL_BUILD=0; shift 1 ;;
        -h|--help) sed -n 's/^# \?//p' "$0" | head -40; exit 0 ;;
        *) _die "unknown option: $1" ;;
    esac
done

[ -n "$PR" ] || _die "--pr is required"
[ -n "$REPO" ] || _die "--repo is required"

# Resolve the guardrails helper (sibling of this script; test override allowed).
_GUARDRAILS="${AUTOSPEC_GUARDRAILS_SH:-}"
if [ -z "$_GUARDRAILS" ]; then
    _dir="$(cd "$(dirname "$0")" && pwd)"
    _GUARDRAILS="$_dir/autonomous-guardrails.sh"
fi
[ -f "$_GUARDRAILS" ] || _die "guardrails helper not found: $_GUARDRAILS"

_TMPDIR="$(mktemp -d -t guarded-merge.XXXXXX)"
_cleanup() { rm -rf "$_TMPDIR" 2>/dev/null || true; }

# 1. Fetch the PR's actual changed files. gh failure is fail-closed.
_changed="$_TMPDIR/changed.txt"
if ! gh pr view "$PR" --repo "$REPO" --json files \
        --jq '.files[].path' > "$_changed" 2>/dev/null; then
    _cleanup
    _die "could not read changed files for PR #$PR (fail-closed, not merged)"
fi

# Empty diff → nothing to classify → allow (proceed to merge).
if [ ! -s "$_changed" ]; then
    printf 'guarded-merge: PR #%s has no changed files; nothing fenced\n' "$PR"
else
    # 2. Classify blast radius against the fenced-surfaces registry.
    _blast_out=""
    _blast_rc=0
    if [ -n "$FENCED_SURFACES" ]; then
        _blast_out="$(bash "$_GUARDRAILS" blast-radius --changed-files "$_changed" \
            --fenced-surfaces "$FENCED_SURFACES" 2>&1)" || _blast_rc=$?
    else
        _blast_out="$(bash "$_GUARDRAILS" blast-radius --changed-files "$_changed" \
            2>&1)" || _blast_rc=$?
    fi

    # 3. Branch on the deterministic DECISION line (not exit code alone), so a
    #    classifier error (no DECISION line) is distinguished from a real
    #    quarantine and fails closed.
    if printf '%s\n' "$_blast_out" | grep -q '^DECISION:quarantine'; then
        # Fenced. Honor an explicit override label on the PR.
        _labels="$(gh pr view "$PR" --repo "$REPO" --json labels \
            --jq '.labels[].name' 2>/dev/null || true)"
        if printf '%s\n' "$_labels" | grep -qxF "$OVERRIDE_LABEL"; then
            printf 'guarded-merge: PR #%s touches a fenced surface but carries override label %s; proceeding\n' \
                "$PR" "$OVERRIDE_LABEL"
        else
            _surfaces="$(printf '%s\n' "$_blast_out" | grep '^SURFACE:' || true)"
            # gh pr edit --add-label is broken on this repo (Projects-classic
            # GraphQL deprecation error on repository.pullRequest.projectCards)
            # — go through the Issues API directly instead. Ensure the label
            # exists first (reuses the ensure_label convention above), then
            # apply it. Neither call is swallowed: a failure here must be
            # operator-visible, since a silent failure leaves a quarantined PR
            # with no queryable trace that a human decision is pending. A
            # failed label/comment call NEVER changes the exit 1 / "blocked
            # fenced_surface" verdict below — that is the load-bearing
            # invariant of this fence.
            ensure_label "$HUMAN_LABEL" "$REPO"
            if ! gh api -X POST "repos/$REPO/issues/$PR/labels" -f "labels[]=$HUMAN_LABEL" >/dev/null 2>&1; then
                _warn "could not apply label '$HUMAN_LABEL' to PR #$PR — quarantine is NOT visible via labels; check manually"
            fi
            if ! gh pr comment "$PR" --repo "$REPO" --body "$(printf 'Blocked by the blast-radius domain fence: this PR touches a fenced surface and requires human review before merge.\n\n```\n%s\n```\n\nAdd the `%s` label after review to override.' "${_surfaces:-$_blast_out}" "$OVERRIDE_LABEL")" >/dev/null 2>&1; then
                _warn "could not comment on PR #$PR to explain the quarantine"
            fi
            printf '%s\n' "$_blast_out"
            printf 'blocked fenced_surface\n'
            _cleanup
            exit 1
        fi
    elif printf '%s\n' "$_blast_out" | grep -q '^DECISION:allow'; then
        printf 'guarded-merge: PR #%s blast-radius allowed\n' "$PR"
    else
        # No parseable DECISION — classifier error. Fail closed.
        printf '%s\n' "$_blast_out" >&2
        _cleanup
        _die "blast-radius classifier produced no DECISION for PR #$PR (fail-closed, not merged)"
    fi
fi

# 4. Local buildability gate (issue #4108): build the PR's exact head commit
#    and read the exit status before spending CI-gate time on it. Fail-closed
#    on a failed build; warn-skip only when no local build of the head is
#    possible (the CI-conclusion gate below then remains the authority).
_local_build_gate() {
    # Returns: 0 = build ok (or warn-skip), 1 = head build failed, 2 = hard
    # error. Called in an `if` context, so set -e is inert inside and every
    # probe is guarded explicitly.
    if [ ! -f "$BUILD_DIR/Cargo.toml" ]; then
        if [ "$_BUILD_DIR_EXPLICIT" = "1" ]; then
            printf 'autospec-guarded-merge: --build-dir %s has no Cargo.toml (operator error)\n' "$BUILD_DIR" >&2
            return 2
        fi
        _warn "local-build-skipped: no Cargo.toml in '$BUILD_DIR' (not a Rust workspace)"
        return 0
    fi
    if ! git -C "$BUILD_DIR" rev-parse --git-dir >/dev/null 2>&1; then
        if [ "$_BUILD_DIR_EXPLICIT" = "1" ]; then
            printf 'autospec-guarded-merge: --build-dir %s is not a git worktree (operator error)\n' "$BUILD_DIR" >&2
            return 2
        fi
        _warn "local-build-skipped: '$BUILD_DIR' is not a git worktree; building an unverified state would not be evidence"
        return 0
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        if [ "$_BUILD_DIR_EXPLICIT" = "1" ]; then
            printf 'autospec-guarded-merge: cargo not on PATH (operator error)\n' >&2
            return 2
        fi
        _warn "local-build-skipped: cargo not on PATH"
        return 0
    fi
    if ! _lb_oid="$(gh pr view "$PR" --repo "$REPO" --json headRefOid --jq .headRefOid 2>/dev/null)"; then
        return 2
    fi
    if [ -z "$_lb_oid" ]; then
        return 2
    fi
    if ! git -C "$BUILD_DIR" cat-file -e "${_lb_oid}^{commit}" 2>/dev/null; then
        if [ "$_BUILD_DIR_EXPLICIT" = "1" ]; then
            printf 'autospec-guarded-merge: PR head %s is not present in --build-dir %s (operator error)\n' "${_lb_oid:0:12}" "$BUILD_DIR" >&2
            return 2
        fi
        _warn "local-build-skipped: head commit ${_lb_oid:0:12} is not present in '$BUILD_DIR' (no network fetch attempted); the CI-conclusion gate remains the authority"
        return 0
    fi
    # Build the exact commit being merged. When the checkout already sits at
    # the head, build in place (reuses its incremental target/); otherwise
    # materialize a detached throwaway worktree at the head, sharing the
    # primary worktree's target dir when one exists so the incremental cache
    # still pays.
    _lb_rc=0
    if [ "$(git -C "$BUILD_DIR" rev-parse HEAD 2>/dev/null || true)" = "$_lb_oid" ]; then
        ( cd "$BUILD_DIR" && cargo build --workspace --all-targets ) || _lb_rc=$?
    else
        _lb_wt="$_TMPDIR/localbuild"
        if ! git -C "$BUILD_DIR" worktree add --detach "$_lb_wt" "$_lb_oid" >/dev/null 2>&1; then
            return 2
        fi
        _lb_target=""
        _lb_common="$(cd "$BUILD_DIR" && git rev-parse --git-common-dir 2>/dev/null || true)"
        if [ -n "$_lb_common" ]; then
            case "$_lb_common" in
                /*) _lb_root="$(dirname "$_lb_common")" ;;
                *) _lb_root="$(cd "$BUILD_DIR/$_lb_common/.." && pwd)" ;;
            esac
            if [ -d "$_lb_root/target" ]; then
                _lb_target="$_lb_root/target"
            fi
        fi
        if [ -n "$_lb_target" ]; then
            ( cd "$_lb_wt" && CARGO_TARGET_DIR="$_lb_target" cargo build --workspace --all-targets ) || _lb_rc=$?
        else
            ( cd "$_lb_wt" && cargo build --workspace --all-targets ) || _lb_rc=$?
        fi
        git -C "$BUILD_DIR" worktree remove --force "$_lb_wt" >/dev/null 2>&1 || true
        git -C "$BUILD_DIR" worktree prune >/dev/null 2>&1 || true
    fi
    if [ "$_lb_rc" != "0" ]; then
        printf 'guarded-merge: PR #%s local build of head %s failed (exit status %s)\n' "$PR" "${_lb_oid:0:12}" "$_lb_rc"
        return 1
    fi
    printf 'guarded-merge: PR #%s local build of head %s ok (exit status 0)\n' "$PR" "${_lb_oid:0:12}"
    return 0
}

_lb_gate_rc=0
if [ "$REQUIRE_LOCAL_BUILD" = "1" ]; then
    if _local_build_gate; then
        _lb_gate_rc=0
    else
        # No '!' here: under 'if ! cmd', $? is the negated status (0).
        _lb_gate_rc=$?
    fi
    if [ "$_lb_gate_rc" = "1" ]; then
        gh pr comment "$PR" --repo "$REPO" --body "$(printf 'Refused by the merge-time local-build gate: `cargo build --workspace --all-targets` against the PR head commit exited non-zero. Not merged.\n\nFix the build, or re-run the merge with `--no-require-local-build` if the failure is known-unrelated and accepted.')" >/dev/null 2>&1 || true
        printf 'blocked local_build_failed\n'
        _cleanup
        exit 1
    elif [ "$_lb_gate_rc" = "2" ]; then
        _cleanup
        _die "local buildability gate error for PR #$PR (fail-closed, not merged)"
    fi
fi

# 5. CI-conclusion gate: refuse while any non-advisory check is pending or not
#    green. A null conclusion means "still running"; counting it as success is
#    exactly how a merge races its own CI run.
_ADVISORY="${AUTOSPEC_PR_ADVISORY_CHECKS:-${AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS:-^$}}"

_rollup_counts() {
    # Emits "<pending> <bad> <total>" for non-advisory entries. A CheckRun
    # carries .conclusion; a StatusContext carries .state — honor both, or a
    # legacy status context reads as pending forever and stalls every merge.
    printf '%s' "$1" | jq -r --arg adv "$_ADVISORY" '
        [ .[]
          | select((((.name // .context // "") as $n
                     | $n != "" and ($n | test($adv)))) | not) ]
        | (map(select((.conclusion // .state) == null
              or ((.conclusion // .state) | ascii_upcase
                  | . == "PENDING" or . == "EXPECTED"
                    or . == "QUEUED" or . == "IN_PROGRESS"))) | length) as $pending
        | (map(select((.conclusion // .state) != null
              and ((.conclusion // .state) | ascii_upcase
                   | . == "FAILURE" or . == "CANCELLED" or . == "TIMED_OUT"
                     or . == "ACTION_REQUIRED" or . == "ERROR"
                     or . == "STARTUP_FAILURE"))) | length) as $bad
        | "\($pending) \($bad) \(length)"'
}

if [ "$REQUIRE_CHECKS" = "1" ]; then
    _deadline=$(( $(date +%s) + CHECKS_TIMEOUT ))
    while :; do
        if ! _rollup="$(gh pr view "$PR" --repo "$REPO" --json statusCheckRollup \
                --jq '.statusCheckRollup // []' 2>/dev/null)"; then
            _cleanup
            _die "could not read the check rollup for PR #$PR (fail-closed, not merged)"
        fi
        if ! _counts="$(_rollup_counts "$_rollup")"; then
            _cleanup
            _die "could not parse the check rollup for PR #$PR (fail-closed, not merged)"
        fi
        _pending="${_counts%% *}"
        _rest="${_counts#* }"
        _bad="${_rest%% *}"
        _total="${_rest##* }"

        if [ "$_bad" != "0" ]; then
            gh pr comment "$PR" --repo "$REPO" --body "$(printf 'Refused by the merge-time CI gate: %s non-advisory check(s) are not green. Not merged.\n\nFix the failing check, or re-run the merge with `--no-require-checks` if the failure is known-unrelated and accepted.' "$_bad")" >/dev/null 2>&1 || true
            printf 'guarded-merge: PR #%s has %s non-advisory check(s) not green\n' "$PR" "$_bad"
            printf 'blocked checks_not_green\n'
            _cleanup
            exit 1
        fi
        if [ "$_total" != "0" ] && [ "$_pending" = "0" ]; then
            printf 'guarded-merge: PR #%s checks green (%s non-advisory)\n' "$PR" "$_total"
            break
        fi
        # An empty rollup is not proof of green: checks may not have registered
        # yet on a freshly pushed head.
        if [ "$(date +%s)" -ge "$_deadline" ]; then
            printf 'guarded-merge: PR #%s still has %s pending / %s total check(s) after %ss\n' \
                "$PR" "$_pending" "$_total" "$CHECKS_TIMEOUT"
            printf 'blocked checks_not_green\n'
            _cleanup
            exit 1
        fi
        sleep "$CHECKS_POLL"
    done
fi

# 6. Full-suite evidence gate (issue #3523): refuse when the recorded
#    full-suite evidence names a different commit, or a failing run. An absent
#    file warns and merges (inert until Phase 4 records); a present file is
#    binding. An unparseable file fails closed like every other read failure.
if [ -n "$VERIFY_EVIDENCE" ]; then
    if [ ! -f "$VERIFY_EVIDENCE" ]; then
        _warn "verify-evidence-absent: $VERIFY_EVIDENCE does not exist; merging without full-suite evidence"
    elif ! jq -e . "$VERIFY_EVIDENCE" >/dev/null 2>&1; then
        printf 'guarded-merge: PR #%s verify-evidence file %s is not valid JSON; not merged\n' "$PR" "$VERIFY_EVIDENCE"
        printf 'blocked verify_evidence_unparseable\n'
        _cleanup
        _die "verify-evidence file is unparseable (fail-closed, not merged): $VERIFY_EVIDENCE"
    else
        _ev_sha="$(jq -r '.head_sha // empty' "$VERIFY_EVIDENCE")"
        _ev_status="$(jq -r '.status // empty' "$VERIFY_EVIDENCE")"
        if [ -z "$_ev_status" ] || ! printf '%s' "$_ev_status" | grep -qE '^[0-9]+$'; then
            printf 'guarded-merge: PR #%s verify-evidence status is missing or non-numeric; not merged\n' "$PR"
            printf 'blocked verify_evidence_failing\n'
            _cleanup
            exit 1
        fi
        if [ "$_ev_status" != "0" ]; then
            printf 'guarded-merge: PR #%s verify-evidence records a failing full suite (status %s); not merged\n' "$PR" "$_ev_status"
            printf 'blocked verify_evidence_failing\n'
            _cleanup
            exit 1
        fi
        if ! _head_oid="$(gh pr view "$PR" --repo "$REPO" --json headRefOid --jq .headRefOid 2>/dev/null)"; then
            _cleanup
            _die "could not read the PR head OID for PR #$PR (fail-closed, not merged)"
        fi
        if [ "$_ev_sha" != "$_head_oid" ]; then
            printf 'guarded-merge: PR #%s verify-evidence head_sha %s != PR head %s (stale); not merged\n' \
                "$PR" "${_ev_sha:-<missing>}" "$_head_oid"
            printf 'blocked verify_evidence_stale\n'
            _cleanup
            exit 1
        fi
        printf 'guarded-merge: PR #%s verify-evidence fresh (head %s, status 0)\n' "$PR" "$_head_oid"
    fi
fi

# 7. Allowed (or overridden) and green: perform the admin merge.
_cleanup
# shellcheck disable=SC2086
gh pr merge "$PR" --repo "$REPO" $MERGE_ARGS
printf 'merged fenced_surface_ok\n'
